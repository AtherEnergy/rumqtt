use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};
use std::net::Shutdown;
use std::collections::VecDeque;
use std::thread;
use std::io::{self, Write, Read, ErrorKind};
use std::result::Result as StdResult;
use std::error::Error as StdError;
use std::fs::File;
use std::path::Path;

use mqtt311::{self, MqttWrite, MqttRead, PacketIdentifier, Packet, Connect, Connack, Protocol, ConnectReturnCode};
use threadpool::ThreadPool;
use jsonwebtoken::{encode, Header, Algorithm};
use chrono::{self, Utc};

use error::{Result, PublishError, PingError, IncomingError, AwaitError, RetransmissionError};
use stream::NetworkStream;
use clientoptions::MqttOptions;
use callback::{Message, MqttCallback};
use super::MqttState;

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    iat: i64,
    exp: i64,
    aud: String,
}

#[derive(Debug)]
pub enum PublishRequest {
    Publish(Box<Message>),
    Shutdown,
    Disconnect,
}

pub struct Publisher {
    opts: MqttOptions,
    stream: NetworkStream,
    nw_request_rx: Receiver<PublishRequest>,
    state: MqttState,
    initial_connect: bool,
    await_pingresp: bool,
    last_flush: Instant,
    last_pkid: PacketIdentifier,
    callback: Option<MqttCallback>,
    outgoing_pub: VecDeque<(Box<Message>)>,
    no_of_reconnections: u32,

    publish_batch_count: u32,
    // thread pool to execute puback callbacks
    pool: ThreadPool,
}

macro_rules! reconnect_loop {
    ($publisher:ident) => ({
        'reconnect: loop {
                match $publisher.try_reconnect() {
                    Ok(_) => {
                        if let Err(e) = $publisher.await() {
                            match e {
                                AwaitError::Reconnect => continue 'reconnect,
                                AwaitError::Io(_) => continue 'reconnect,
                            }
                        } else {
                            break 'reconnect
                        }
                    }
                    Err(e) => {
                        error!("Try Reconnect Failed. Error = {:?}", e);
                        continue 'reconnect;
                    }
                }
            }
    })
}

impl Publisher {
    pub fn connect(opts: MqttOptions, nw_request_rx: Receiver<PublishRequest>, callback: Option<MqttCallback>) -> Result<Self> {

        let mut publisher = Publisher {
            opts: opts,
            stream: NetworkStream::None,
            nw_request_rx: nw_request_rx,
            state: MqttState::Disconnected,
            initial_connect: true,
            await_pingresp: false,
            last_flush: Instant::now(),
            last_pkid: PacketIdentifier(0),

            outgoing_pub: VecDeque::new(),

            callback: callback,
            no_of_reconnections: 0,
            publish_batch_count: 0,
            pool: ThreadPool::new(1),
        };

        if publisher.opts.first_reconnection_loop {
            reconnect_loop!(publisher);
        } else {
            // Make initial tcp connection, send connect packet and
            // return if connack packet has errors. Doing this here
            // ensures that user doesn't have access to this object
            // before mqtt connection
            publisher.try_reconnect()?;
            publisher.await().unwrap();
        }

        Ok(publisher)
    }

    pub fn mock_connect(opts: MqttOptions) -> Self {
        let (_tx, rx) = sync_channel::<PublishRequest>(1);

        Publisher {
            opts: opts,
            stream: NetworkStream::None,
            nw_request_rx: rx,
            state: MqttState::Disconnected,
            initial_connect: true,
            await_pingresp: false,
            last_flush: Instant::now(),
            last_pkid: PacketIdentifier(0),

            outgoing_pub: VecDeque::new(),

            callback: None,
            no_of_reconnections: 0,
            publish_batch_count: 0,
            pool: ThreadPool::new(1),
        }
    }

    pub fn run(&mut self) -> Result<()> {
        let timeout = Duration::new(self.opts.keep_alive.unwrap() as u64, 0);

        loop {
            'publisher: loop {
                let pr = match self.nw_request_rx.recv_timeout(timeout) {
                    Ok(v) => v,
                    Err(RecvTimeoutError::Timeout) => {
                        // Always ping when there are no publish requests. Not relying on last_flush_time
                        if let Err(e) = self.ping() {
                            error!("Ping error. Error = {:?}", e);
                            break 'publisher
                        }

                        // if publish requests stop before batch count is full, they
                        // are awaited during ping
                        if self.publish_batch_count > 0 {
                            info!("Awaiting for publishes in timeout. Publish batch count = {}", self.publish_batch_count);
                            if let Err(e) = self.batch_await() {
                                match e {
                                    AwaitError::Reconnect => break 'publisher,
                                    AwaitError::Io(_) => break 'publisher,
                                }
                            }
                        }

                        // await for ping
                        if let Err(e) = self.await() {
                            match e {
                                AwaitError::Reconnect => break 'publisher,
                                AwaitError::Io(_) => break 'publisher,
                            }
                        }

                        continue 'publisher
                    }
                    Err(e) => {
                        error!("Publisher recv error. Error = {:?}", e);
                        return Err(e.into());
                    }
                };

                match pr {
                    PublishRequest::Shutdown => self.stream.shutdown(Shutdown::Both)?,
                    PublishRequest::Disconnect => self.disconnect()?,
                    PublishRequest::Publish(m) => {
                        if let Err(e) = self.publish(m) {
                            error!("Publish error. Error = {:?}", e);
                            match e {
                                PublishError::Io(_) => break 'publisher,
                                PublishError::PacketSizeLimitExceeded => continue 'publisher,
                                PublishError::InvalidState => break 'publisher,
                            }
                        }

                        // you'll know of disconnections immediately here even when writes
                        // doesn't error out immediately after disonnection
                        if self.publish_batch_count >= self.opts.await_batch_size {
                            if let Err(e) = self.batch_await() {
                                match e {
                                    AwaitError::Reconnect => break 'publisher,
                                    AwaitError::Io(_) => break 'publisher,
                                }
                            }
                        }
                    }
                };
            }

            reconnect_loop!(self)
        }
    }

    fn batch_await(&mut self) -> StdResult<(), AwaitError> {
        for _ in 0..self.publish_batch_count {
            self.await()?
        };

        self.publish_batch_count = 0;
        Ok(())
    }

    // Awaits for an incoming packet and handles internal states appropriately
    fn await(&mut self) -> StdResult<(), AwaitError> {
        let packet = self.stream.read_packet();

        if let Ok(packet) = packet {
            if let Err(e) = self.handle_packet(packet) {
                error!("Handle packet error = {:?}.", e);
                Err(AwaitError::Reconnect)
            } else {
                Ok(())
            }
        } else if let Err(mqtt311::Error::Io(e)) = packet {
            match e.kind() {
                ErrorKind::TimedOut | ErrorKind::WouldBlock => {
                    error!("Timeout waiting for incoming ack. Error = {:?}", e);
                    error!("Publish batch count = {:?}", self.publish_batch_count);
                    Err(AwaitError::Io(e))
                }
                _ => {
                    // Socket error are readily available here as soon as
                    // broker closes its socket end. (But not inbetween n/w disconnection
                    // and socket close at broker [i.e ping req timeout])

                    // UPDATE: Lot of publishes are being written by the time this notified
                    // the eventloop thread. Setting disconnect_block = true during write failure
                    error!("* Error receiving packet. Error = {:?}", e);
                    Err(AwaitError::Io(e))
                }
            }
        } else {
            error!("** Error receiving packet. Error = {:?}", packet);
            Err(AwaitError::Reconnect)
        }
    }

    /// Creates a Tcp Connection, Sends Mqtt connect packet and sets state to
    /// Handshake mode if Tcp write and Mqtt connect succeeds
    fn try_reconnect(&mut self) -> Result<()> {
        self.unbind();

        if !self.initial_connect {
            error!("  Will try Reconnect in {} seconds", self.opts.reconnect);
            thread::sleep(Duration::new(self.opts.reconnect as u64, 0));
        }

        self.initial_connect = false;
        let host_name_verification = self.opts.host_name_verification;
        let mut stream = NetworkStream::connect(&self.opts.addr, self.opts.ca.clone(), self.opts.client_certs.clone(), host_name_verification)?;

        //NOTE: Should be less than default keep alive time to make sure that server doesn't 
        //      disconnect while waiting for read.
        stream.set_read_timeout(Some(Duration::new(10, 0)))?;
        stream.set_write_timeout(Some(Duration::new(30, 0)))?;

        if let Some((ref key, expiry)) = self.opts.gcloud_iotcore_auth {
            let password = gen_password(key, expiry);
            self.opts.credentials = Some(("unused".to_owned(), password));
        }

        self.stream = stream;
        let connect = self.generate_connect_packet();
        let connect = Packet::Connect(connect);
        self.write_packet(connect)?;
        self.state = MqttState::Handshake;
        Ok(())
    }

    fn generate_connect_packet(&self) -> Box<Connect> {
        let keep_alive = if let Some(dur) = self.opts.keep_alive {
            dur
        } else {
            0
        };

        Box::new(
            Connect {
                protocol: Protocol::MQTT(4),
                keep_alive: keep_alive,
                client_id: self.opts.client_id.clone().expect("No Client Id"),
                clean_session: self.opts.clean_session,
                last_will: None,
                username: self.opts.credentials.clone().map(|u| u.0),
                password: self.opts.credentials.clone().map(|p| p.1),
            }
        )
    }

    fn handle_packet(&mut self, packet: Packet) -> StdResult<(), IncomingError> {
        match self.state {
            MqttState::Handshake => {
                if let Packet::Connack(connack) = packet {
                    self.handle_connack(connack)
                } else {
                    error!("Invalid Packet in Handshake State --> {:?}", packet);
                    Err(IncomingError::ConnectionAbort)
                }
            }
            MqttState::Connected => {
                match packet {
                    Packet::Pingresp => {
                        self.await_pingresp = false;
                        info!("Received ping resp");
                        Ok(())
                    }
                    Packet::Disconnect => Ok(()),
                    Packet::Puback(puback) => self.handle_puback(puback),
                    _ => {
                        error!("Invalid Packet in Connected State --> {:?}", packet);
                        Err(IncomingError::ConnectionAbort)
                    }
                }
            }
            MqttState::Disconnected => {
                error!("Invalid Packet in Disconnected State --> {:?}", packet);
                Err(IncomingError::ConnectionAbort)
            }
        }
    }

    /// Checks Mqtt connack packet's status code and sets Mqtt state
    /// to `Connected` if successful
    fn handle_connack(&mut self, connack: Connack) -> StdResult<(), IncomingError> {
        let code = connack.code;

        if code != ConnectReturnCode::Accepted {
            error!("Failed to connect. Error = {:?}", code);
            return Err(IncomingError::MqttConnectionRefused(code));
        }

        if self.initial_connect {
            self.initial_connect = false;
        } else {
            self.no_of_reconnections += 1;
        }

        info!("Mqtt connection successful !!");
        self.state = MqttState::Connected;

        // Retransmit QoS1,2 queues after reconnection when clean_session = false
        if !self.opts.clean_session {
            if self.force_retransmit().is_err() {
                return Err(IncomingError::ConnectionAbort)
            }
        }

        Ok(())
    }

    // NOTE: Using different error type instead of bigger general error so that the caller
    //       can use the typesystem to cover all possible error cases
    fn publish(&mut self, publish_message: Box<Message>) -> StdResult<(), PublishError> {
        // Assign next pkid only when pkid is None because spec says re-transmission
        // should be done using same pkid
        let publish_message = if publish_message.pkid.is_some(){
            publish_message
        }else {
            publish_message.set_pkid(Some(self.next_pkid()))
        };

        let payload_len = publish_message.payload.len();

        if payload_len > self.opts.storepack_sz {
            warn!("Size limit exceeded. Dropping packet: {:?}", publish_message);
            return Err(PublishError::PacketSizeLimitExceeded)
        } else {
            self.outgoing_pub.push_back(publish_message.clone());
        }
        
        if self.outgoing_pub.len() > 50 * 50 {
            warn!(":( :( Outgoing publish queue length growing bad --> {:?}", self.outgoing_pub.len());
        }

        let packet = Packet::Publish(publish_message.to_mqtt_message().to_pub(None, false));

        if self.state == MqttState::Connected {
            self.write_packet(packet)?;
            info!("Published. Pkid = {:?}, Payload Size = {:?}", publish_message.pkid, payload_len);
            self.publish_batch_count += 1;
        } else {
            error!("State = {:?}. Should'nt publish in this state", self.state);
            return Err(PublishError::InvalidState)
        }

        Ok(())
    }

    fn handle_puback(&mut self, pkid: PacketIdentifier) -> StdResult<(), IncomingError> {
        // debug!("*** PubAck --> Pkid({:?})\n--- Publish Queue =\n{:#?}\n\n", pkid, self.outgoing_pub);
        debug!("Received puback for: {:?}", pkid);

        let m = match self.outgoing_pub
                          .iter()
                          .position(|x| x.pkid == Some(pkid)) {
            Some(i) => {
                if let Some(m) = self.outgoing_pub.remove(i) {
                    Some(*m)
                } else {
                    None
                }
            }
            None => {
                error!("Oopssss..unsolicited ack --> {:?}", pkid);
                None
            }
        };

        if let Some(val) = m {
            if let Some(ref callback) = self.callback {
                if let Some(ref on_publish) = callback.on_publish {
                    let on_publish = on_publish.clone();
                    self.pool.execute(move || on_publish(val));
                }
            }
        }

        debug!("Pub Q Len After Ack @@@ {:?}", self.outgoing_pub.len());
        Ok(())
    }

    fn disconnect(&mut self) -> Result<()> {
        let disconnect = Packet::Disconnect;
        self.write_packet(disconnect)?;
        Ok(())
    }

    fn ping(&mut self) -> StdResult<(), PingError> {
        if let Some(_) = self.opts.keep_alive {

            // @ Prevents half open connections. Tcp writes will buffer up
            // with out throwing any error (till a timeout) when internet
            // is down. Eventhough broker closes the socket, EOF will be
            // known only after reconnection.
            // We just unbind the socket if there in no pingresp before next ping
            // (What about case when pings aren't sent because of constant publishes
            // ?. A. Tcp write buffer gets filled up and write will be blocked for 10
            // secs and then error out because of timeout.)
            if self.await_pingresp {
                return Err(PingError::AwaitPingResp);
            }
            let ping = Packet::Pingreq;
            self.await_pingresp = true;

            info!("Rumqtt ping!! await_ping = {}", self.await_pingresp);

            if self.state == MqttState::Connected {
                self.write_packet(ping)?;
            } else {
                error!("State = {:?}. Shouldn't ping in this state", self.state);
                return Err(PingError::InvalidState)
            }
        }

        Ok(())
    }

    // Spec says that client (for QoS > 0, persistant session [clean session = 0])
    // should retransmit all the unacked publishes and pubrels after reconnection.
    fn force_retransmit(&mut self) -> StdResult<(), RetransmissionError> {
        // Cloning because iterating and removing isn't possible.
        // Iterating over indexes and and removing elements messes
        // up the remove sequence
        let mut outgoing_pub = self.outgoing_pub.clone();
        // debug!("*** Force Retransmission. Publish Queue =\n{:#?}\n\n", outgoing_pub);
        self.outgoing_pub.clear();

        while let Some(message) = outgoing_pub.pop_front() {
            if let Err(e) = self.publish(message) {
                error!("Publish error during retransmission. Skipping. Error = {:?}", e);
                continue
            }

            // TODO: Await errors might result in clean queue.
            if self.publish_batch_count >= self.opts.await_batch_size {
                self.batch_await()?;
            }
        }

        Ok(())
    }

    fn unbind(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
        self.await_pingresp = false;
        self.state = MqttState::Disconnected;
        self.publish_batch_count = 0;

        // remove all the state
        if self.opts.clean_session {
            self.outgoing_pub.clear();
        }
    }

    // http://stackoverflow.com/questions/11115364/mqtt-messageid-practical-implementation
    #[inline]
    fn next_pkid(&mut self) -> PacketIdentifier {
        let PacketIdentifier(mut pkid) = self.last_pkid;
        if pkid == 65535 {
            pkid = 0;
        }
        self.last_pkid = PacketIdentifier(pkid + 1);
        self.last_pkid
    }

    // NOTE: write_all() will block indefinitely by default if
    // underlying Tcp Buffer is full (during disconnections). This
    // is evident when test cases are publishing lot of data when
    // ethernet cable is unplugged (mantests/half_open_publishes_and_reconnections
    // but not during mantests/ping_reqs_in_time_and_reconnections due to low
    // frequency writes. 60 seconds migth be good default for write timeout ?)
    // https://stackoverflow.com/questions/11037867/socket-send-call-getting-blocked-for-so-long
    fn write_packet(&mut self, packet: Packet) -> io::Result<()> {
        if let Err(e) = self.stream.write_packet(&packet) {
            error!("Write error = {:?}", e);
            return Err(io::Error::new(ErrorKind::Other, e.description()))
        }
        self.flush()?;
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()?;
        self.last_flush = Instant::now();
        Ok(())
    }
}

// Generates a new password for mqtt client authentication
pub fn gen_password<P>(key: P, expiry: i64) -> String
where P: AsRef<Path> {
    let time = Utc::now();
    let jwt_header = Header::new(Algorithm::RS256);
    let iat = time.timestamp();
    let exp = time.checked_add_signed(chrono::Duration::minutes(expiry)).unwrap().timestamp();
    let claims = Claims {
        iat: iat,
        exp: exp,
        aud: "crested-return-122311".to_string(),
    };

    let mut key_file = File::open(key).expect("Unable to open private keyfile for gcloud iot core auth");
    let mut key = vec![];
    key_file.read_to_end(&mut key).expect("Unable to read private key file for gcloud iot core auth till end");
    encode(&jwt_header, &claims, &key).expect("encode error")
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use publisher::Publisher;
    use mqtt311::{PacketIdentifier, Connack, ConnectReturnCode};
    use clientoptions::MqttOptions;
    use callback::Message;
    use error::{PublishError, IncomingError};
    use MqttState;

    #[test]
    fn next_pkid_roll() {
        let opts = MqttOptions::new();
        let mut connection = Publisher::mock_connect(opts);
        let mut pkt_id = PacketIdentifier(0);
        for _ in 0..65536 {
            pkt_id = connection.next_pkid();
        }
        assert_eq!(PacketIdentifier(1), pkt_id);
     }

     #[test]
     fn unbind_behaviour_during_clean_and_persistent_sessions() {
        // persistent session
        let opts = MqttOptions::new();
        let mut connection = Publisher::mock_connect(opts);
        connection.state = MqttState::Connected;
        connection.await_pingresp = true;
        connection.publish_batch_count = 100;

        let publish = Message {
            topic: "a/b".to_string(),
            payload: Arc::new(vec![1, 2, 3]),
            pkid: None,
            userdata: None,
        };
        let publish = Box::new(publish);

        connection.outgoing_pub.push_back(publish.clone());
        connection.outgoing_pub.push_back(publish.clone());

        connection.unbind();
        assert_eq!(connection.outgoing_pub.len(), 2);
        assert_eq!(connection.state, MqttState::Disconnected);
        assert_eq!(connection.await_pingresp, false);

        // clean session
        let opts = MqttOptions::new().set_clean_session(true);
        let mut connection = Publisher::mock_connect(opts);
        connection.state = MqttState::Connected;
        connection.await_pingresp = true;

        let publish = Message {
            topic: "a/b".to_string(),
            payload: Arc::new(vec![1, 2, 3]),
            pkid: None,
            userdata: None,
        };
        let publish = Box::new(publish);

        connection.outgoing_pub.push_back(publish.clone());
        connection.outgoing_pub.push_back(publish.clone());

        connection.unbind();
        assert_eq!(connection.outgoing_pub.len(), 0);
        assert_eq!(connection.state, MqttState::Disconnected);
        assert_eq!(connection.await_pingresp, false);
        assert_eq!(connection.publish_batch_count, 0);
     }

     /// makes sure that all the messages cleared from the queue during retransmission
     /// are put in the queue agian. even during publish errors
     #[test]
     fn force_retransmission_should_put_everything_back_to_queue_during_errors() {
        // persistent session
        let opts = MqttOptions::new();
        let mut connection = Publisher::mock_connect(opts);
        connection.state = MqttState::Connected;
        connection.await_pingresp = true;

        let publish = Message {
            topic: "a/b".to_string(),
            payload: Arc::new(vec![1, 2, 3]),
            pkid: None,
            userdata: None,
        };
        let publish = Box::new(publish);

        connection.outgoing_pub.push_back(publish.clone());
        connection.outgoing_pub.push_back(publish.clone());

        let _ = connection.force_retransmit();
        assert_eq!(connection.outgoing_pub.len(), 2);
     }

     #[test]
     fn publish_should_return_error_when_payload_size_exceeds_threshold() {
         // persistent session
        let opts = MqttOptions::new();
        let mut connection = Publisher::mock_connect(opts);
        connection.state = MqttState::Connected;
        connection.await_pingresp = true;

        let publish = Message {
            topic: "a/b".to_string(),
            payload: Arc::new(vec![0; 101 * 1024]),
            pkid: None,
            userdata: None,
        };
        let publish = Box::new(publish);

        if let Err(PublishError::PacketSizeLimitExceeded) = connection.publish(publish) {
            ()
        } else {
            panic!("Should return PacketSizeLimitExceeded error");
        }
     }

     #[test]
     fn publish_should_return_error_while_publishing_in_non_connected_state() {
        // persistent session
        let opts = MqttOptions::new();
        let mut connection = Publisher::mock_connect(opts);
        connection.state = MqttState::Handshake;
        connection.await_pingresp = true;

        let publish = Message {
            topic: "a/b".to_string(),
            payload: Arc::new(vec![1, 2, 3]),
            pkid: None,
            userdata: None,
        };
        let publish = Box::new(publish);

        if let Err(PublishError::InvalidState) = connection.publish(publish) {
            ()
        } else {
            panic!("Should return InvalidState error");
        }
     }

     #[test]
     fn pubacks_should_clear_queues() {
        let opts = MqttOptions::new();
        let mut connection = Publisher::mock_connect(opts);
        connection.state = MqttState::Connected;
        connection.await_pingresp = true;

        let publish = Message {
            topic: "a/b".to_string(),
            payload: Arc::new(vec![1, 2, 3]),
            pkid: None,
            userdata: None,
        };
        let publish = Box::new(publish);

        let _ = connection.publish(publish.clone());
        let _ = connection.publish(publish.clone());
        let _ = connection.publish(publish.clone());

        for i in 0..connection.outgoing_pub.len() {
            assert_eq!(connection.outgoing_pub[i].pkid.unwrap(), PacketIdentifier(i as u16 + 1));
        }

        for i in 0..connection.outgoing_pub.len() {
            let _ = connection.handle_puback(PacketIdentifier(i as u16 + 1));
        }

        assert_eq!(connection.outgoing_pub.len(), 0);
     }

     #[test]
     fn connection_should_error_for_errored_connack() {
        let opts = MqttOptions::new();
        let mut connection = Publisher::mock_connect(opts);
        connection.state = MqttState::Disconnected;

        let connack = Connack{session_present: false, code: ConnectReturnCode::BadUsernamePassword};
        if let Err(IncomingError::MqttConnectionRefused(e)) = connection.handle_connack(connack) {
            assert_eq!(e, ConnectReturnCode::BadUsernamePassword);
        } else {
            panic!("Should error with 'BadUsernamePassword'");
        }
     }

     #[test]
     fn connack_should_set_correct_state() {
        let opts = MqttOptions::new();
        let mut connection = Publisher::mock_connect(opts);
        connection.state = MqttState::Disconnected;

        let connack = Connack{session_present: false, code: ConnectReturnCode::Accepted};
        let _ = connection.handle_connack(connack);

        assert_eq!(MqttState::Connected, connection.state);
        assert_eq!(false, connection.initial_connect);
     }
}
