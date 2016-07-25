use std::time::{Duration, Instant};
use time;

use rand::{self, Rng};
use std::net::{SocketAddr, ToSocketAddrs, Shutdown};
use error::{Error, Result};
use message::Message;
use std::collections::VecDeque;
use std::io::Write;
use std::str;
use mio::tcp::TcpStream;
use mio::*;
use mqtt::{Encodable, Decodable, QualityOfService, TopicFilter};
use mqtt::packet::*;
use mqtt::control::variable_header::{ConnectReturnCode, PacketIdentifier};
use mqtt::topic_name::TopicName;
use std::sync::Arc;
use std::thread;
use tls::{SslContext, NetworkStream};
use std::sync::mpsc::{self, SyncSender};

const MIO_PING_TIMER: u64 = 123;
const MIO_QUEUE_TIMER: u64 = 321;
const MIO_CLIENT_STREAM: Token = Token(1);

// static mut N: i32 = 0;
// unsafe {
//     N += 1;
//     println!("N: {}", N);
// }

#[derive(Clone)]
pub struct MqttOptions {
    keep_alive: Option<u16>,
    clean_session: bool,
    client_id: Option<String>,
    username: Option<String>,
    password: Option<String>,
    reconnect: Option<u16>,
    will: Option<(String, String)>,
    will_qos: QualityOfService,
    will_retain: bool,
    pub_q_len: u16,
    sub_q_len: u16,
    queue_timeout: u16, // wait time for ack beyond which packet(publish/subscribe) will be resent
    ssl: Option<SslContext>,
}

impl Default for MqttOptions {
    fn default() -> Self {
        MqttOptions {
            keep_alive: Some(5),
            clean_session: true,
            client_id: None,
            username: None,
            password: None,
            reconnect: None,
            will: None,
            will_qos: QualityOfService::Level0,
            will_retain: false,
            pub_q_len: 50,
            sub_q_len: 5,
            queue_timeout: 60,
            ssl: None,
        }
    }
}


impl MqttOptions {
    /// Creates a new `MqttOptions` object which is used to set connection
    /// options for new client. Below are defaults with which this object is
    /// created.
    ///
    /// |                         |                          |
    /// |-------------------------|--------------------------|
    /// | **client_id**           | Randomly generated       |
    /// | **clean_session**       | true                     |
    /// | **keep_alive**          | 5 secs                   |
    /// | **reconnect try**       | Doesn't try to reconnect |
    /// | **retransmit try time** | 60 secs                  |
    /// | **pub_q_len**           | 50                       |
    /// | **sub_q_len**           | 5                        |
    ///
    pub fn new() -> MqttOptions { MqttOptions { ..Default::default() } }

    /// Number of seconds after which client should ping the broker
    /// if there is no other data exchange
    pub fn set_keep_alive(&mut self, secs: u16) -> &mut Self {
        self.keep_alive = Some(secs);
        self
    }

    /// Client id of the client. A random client id will be selected

    /// if you don't set one
    pub fn set_client_id(&mut self, client_id: &str) -> &mut Self {
        self.client_id = Some(client_id.to_string());
        self
    }

    /// `clean_session = true` instructs the broker to clean all the client
    /// state when it disconnects. Note that it is broker which is discarding
    /// the client state. But this client will hold its queues and attemts to
    /// to retransmit when reconnection happens.  (TODO: Verify this)
    ///
    /// When set `false`, broker will hold the client state and performs pending
    /// operations on the client when reconnection with same `client_id`
    /// happens.
    ///
    /// Hence **make sure that you manually set `client_id` when
    /// `clean_session` is false**
    pub fn set_clean_session(&mut self, clean_session: bool) -> &mut Self {
        self.clean_session = clean_session;
        self
    }

    fn generate_client_id(&mut self) -> &mut Self {
        let mut rng = rand::thread_rng();
        let id = rng.gen::<u32>();
        self.client_id = Some(format!("mqttc_{}", id));
        self
    }

    /// Set `username` for broker to perform client authentication
    /// via `username` and `password`
    pub fn set_user_name(&mut self, username: &str) -> &mut Self {
        self.username = Some(username.to_string());
        self
    }

    /// Set `password` for broker to perform client authentication
    /// vis `username` and `password`
    pub fn set_password(&mut self, password: &str) -> &mut Self {
        self.password = Some(password.to_string());
        self
    }

    /// All the `QoS > 0` publishes state will be saved to attempt
    /// retransmits incase ack from broker fails.
    ///
    /// If broker disconnects for some time, `Publisher` shouldn't throw error
    /// immediately during publishes. At the same time, `Publisher` shouldn't be
    /// allowed to infinitely push to the queue.
    ///
    /// Publish queue length specifies maximum queue capacity upto which
    /// `Publisher`
    /// can push with out blocking. Messages in this queue will published as
    /// soon as
    /// connection is reestablished and `Publisher` gets unblocked
    pub fn set_pub_q_len(&mut self, len: u16) -> &mut Self {
        self.pub_q_len = len;
        self
    }

    pub fn set_sub_q_len(&mut self, len: u16) -> &mut Self {
        self.sub_q_len = len;
        self
    }

    /// Time interval after which client should retry for new
    /// connection if there are any disconnections.
    /// By default, no retry will happen
    pub fn set_reconnect(&mut self, dur: u16) -> &mut Self {
        self.reconnect = Some(dur);
        self
    }

    /// Set will for the client so that broker can send `will_message`
    /// on `will_topic` when this client ungracefully dies.
    pub fn set_will(&mut self, will_topic: &str, will_message: &str) -> &mut Self {
        self.will = Some((will_topic.to_string(), will_message.to_string()));
        self
    }

    /// Set QoS for the will message
    pub fn set_will_qos(&mut self, qos: QualityOfService) -> &mut Self {
        self.will_qos = qos;
        self
    }

    /// Set will retian so that future clients subscribing to will topic
    /// knows of client's death.
    pub fn set_will_retain(&mut self, retain: bool) -> &mut Self {
        self.will_retain = retain;
        self
    }

    /// Set a TLS connection
    pub fn set_tls(&mut self, ssl: SslContext) -> &mut Self {
        self.ssl = Some(ssl);
        self
    }

    fn lookup_ipv4<A: ToSocketAddrs>(addr: A) -> SocketAddr {
        let addrs = addr.to_socket_addrs().expect("Conversion Failed");
        for addr in addrs {
            if let SocketAddr::V4(_) = addr {
                return addr;
            }
        }
        unreachable!("Cannot lookup address");
    }

    /// Creates a new mqtt client with the broker address that you want
    /// to connect to. Along with connection details, this object holds
    /// all the state information of a connection.
    ///
    /// **NOTE**: This should be the final call of `MqttOptions` method
    /// chaining
    ///
    /// ```ignore
    /// let client = client_options.set_keep_alive(5)
    ///                           .set_reconnect(5)
    ///                           .set_client_id("my-client-id")
    ///                           .set_clean_session(true)
    ///                           .connect("localhost:1883");
    ///
    pub fn connect<A: ToSocketAddrs>(&mut self, addr: A) -> ProxyClient {
        if self.client_id == None {
            self.generate_client_id();
        }

        let addr = Self::lookup_ipv4(addr);

        ProxyClient {
            addr: addr,
            stream: NetworkStream::None,

            // State
            last_flush: Instant::now(),
            last_pkid: PacketIdentifier(0),
            await_ping: false,
            state: MqttState::Disconnected,
            initial_connect: true,
            opts: self.clone(),
            pub1_channel_pending: 0,
            pub2_channel_pending: 0,
            should_qos1_block: false,
            should_qos2_block: false,

            // Channels
            pub0_rx: None,
            pub1_rx: None,
            pub2_rx: None,
            sub_rx: None,
            connsync_tx: None,
            mionotify_tx: None,

            // Queues
            incoming_rec: VecDeque::new(),
            outgoing_pub: VecDeque::new(),
            outgoing_rec: VecDeque::new(),
            outgoing_rel: VecDeque::new(),

            // callback
            callback: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MqttState {
    Handshake,
    Connected,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PubNotify {
    QoS0,
    QoS1,
    QoS1QueueDown,
    QoS1Reconnect,
    QoS2Reconnect,
    QoS2,
    QoS2QueueDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MioNotification {
    Pub(PubNotify),
    Sub,
    Disconnect,
    Shutdown,
}


/// Handles commands from Publisher and Subscriber. Saves MQTT
/// state and takes care of retransmissions.
pub struct ProxyClient {
    addr: SocketAddr,
    opts: MqttOptions,
    stream: NetworkStream,

    // state
    state: MqttState,
    last_flush: Instant,
    last_pkid: PacketIdentifier,
    await_ping: bool,
    initial_connect: bool,
    pub1_channel_pending: u32,
    pub2_channel_pending: u32,
    should_qos1_block: bool,
    should_qos2_block: bool,

    // Channels
    pub0_rx: Option<mpsc::Receiver<Message>>,
    pub1_rx: Option<mpsc::Receiver<Message>>,
    pub2_rx: Option<mpsc::Receiver<Message>>,
    sub_rx: Option<mpsc::Receiver<Vec<(TopicFilter, QualityOfService)>>>,
    connsync_tx: Option<mpsc::SyncSender<MqttStatus>>,
    mionotify_tx: Option<Sender<MioNotification>>,

    /// Queues. Note: 'record' is qos2 term for 'publish'
    /// For QoS 1. Stores outgoing publishes
    outgoing_pub: VecDeque<(i64, Box<Message>)>,
    /// For QoS 2. Store for incoming publishes to record.
    incoming_rec: VecDeque<Box<Message>>, //
    /// For QoS 2. Store for outgoing publishes.
    outgoing_rec: VecDeque<(i64, Box<Message>)>,
    /// For Qos2. Store for outgoing `pubrel` packets.
    outgoing_rel: VecDeque<(i64, PacketIdentifier)>,

    /// On message callback
    callback: Option<Arc<SendableFn>>,
}


impl Handler for ProxyClient {
    type Timeout = u64;
    type Message = MioNotification;

    // TODO: Understand --> when connecting to localbroker, For Incoming publishes,
    // state is at Readable@Connected state but while connecting to remote broker,
    // state is at Writable@Connected. Hence ended up add STATE_reate,
    // STATE_handle_packet
    // to both readable and writable
    fn ready(&mut self, event_loop: &mut EventLoop<ProxyClient>, token: Token, events: EventSet) {
        if events.is_readable() {
            info!("@@@ Readable ... {:?}", self.state);
            match token {
                MIO_CLIENT_STREAM => {
                    match self.state {
                        MqttState::Connected | MqttState::Handshake => {
                            let packet = match self.STATE_read_incoming(event_loop) {
                                Ok(pk) => pk,
                                Err(_) => return,
                            };


                            trace!("{:?}", packet);
                            self.STATE_handle_packet(&packet, event_loop);

                        }
                        MqttState::Disconnected => {
                            // Handles case where initial connection failures
                            // E.g testsuite --> inital_mqtt_connect_failure_test
                            if self.initial_connect == true {
                                match self.connsync_tx {
                                    Some(ref connsync_tx) => {
                                        connsync_tx.send(MqttStatus::Failed).expect("Send failed");
                                        self.initial_connect = false;
                                    }
                                    None => panic!("No connsync channel"),
                                }
                            } else {
                                self.STATE_try_reconnect(event_loop);
                            }
                        }
                    }
                }

                _ => panic!("Invalid token .."),
            }
        }

        // You do not need to register EventSet::writable() if you want to immediately
        // write back
        // to the socket you just read from. You can just perform the write as part of
        // the current
        // readable event. If you are performing an expensive task between the read and
        // write, you may want to handle this differently. Whether you are reading or
        // writing from
        // the socket, you also need to be aware that the kernel might not be ready for
        // your read
        // r write.
        //
        // NOTE: THIS IS BEING INVOKED AFTER TCPSTREAM::CONNECT (server up or not)
        // It might be READABLE for new incoming connections (for server) or data
        if events.is_writable() {
            info!("%%% Writable ... {:?}", self.state);
            match token {
                MIO_CLIENT_STREAM => {
                    match self.state {
                        MqttState::Disconnected => {
                            debug!("sending mqtt connect packet ..");
                            match self._connect() {
                                // @ If writing connect packet is successful means TCP
                                // @ connection is successful. Change the state machine
                                // @ to handshake and event loop to readable to read
                                // @ CONACK packet
                                Ok(_) => {
                                    self.state = MqttState::Handshake;
                                    event_loop.reregister(self.stream.get_ref().unwrap(),
                                            MIO_CLIENT_STREAM,
                                            EventSet::readable(),
                                            PollOpt::edge() | PollOpt::oneshot())
                                    .unwrap();
                                }
                                // @ Writing connect packet failed. Tcp connection might
                                // @ have failed. Try to establish a new Tcp connection
                                // @ and set eventloop to writable to write connect packets
                                // @ again
                                Err(e) => {
                                    error!("Connecion error {:?}", e);
                                    // Handles case where initial connection failures
                                    // E.g testsuite --> inital_mqtt_connect_failure_test
                                    if self.initial_connect == true {
                                        match self.connsync_tx {
                                            Some(ref connsync_tx) => {
                                                connsync_tx.send(MqttStatus::Failed).expect("Send failed");
                                                self.initial_connect = false;
                                            }
                                            None => panic!("No connsync channel"),
                                        }
                                    } else {
                                        self.STATE_try_reconnect(event_loop);
                                    }
                                }
                            }
                        }
                        MqttState::Handshake | MqttState::Connected => {
                            error!("Incoming data in writable state ..");
                            event_loop.reregister(self.stream.get_ref().unwrap(),
                                            MIO_CLIENT_STREAM,
                                            EventSet::readable(),
                                            PollOpt::edge() | PollOpt::oneshot())
                                .unwrap();

                        }
                    }
                }

                _ => panic!("Invalid token .."),
            }
        }
    }

    fn timeout(&mut self, event_loop: &mut EventLoop<Self>, timer: Self::Timeout) {
        match timer {
            MIO_PING_TIMER => {
                debug!("client state --> {:?}, await_ping --> {}", self.state, self.await_ping);

                match self.state {
                    MqttState::Connected => {
                        if !self.await_ping {
                            let _ = self.ping();
                        } else {
                            error!("awaiting for previous ping resp");
                        }

                        if let Some(keep_alive) = self.opts.keep_alive {
                            event_loop.timeout_ms(MIO_PING_TIMER, keep_alive as u64 * 900).unwrap();
                        }
                    }

                    MqttState::Disconnected | MqttState::Handshake => {
                        error!("I won't ping. Client is in disconnected/handshake state")
                    }
                }
            }

            MIO_QUEUE_TIMER => {
                match self.state {
                    MqttState::Connected => {
                        debug!("^^^ QUEUE RESEND");
                        self._try_retransmit();
                    }
                    MqttState::Disconnected | MqttState::Handshake => {
                        debug!("I won't republish. Client is in disconnected/handshake state")
                    }
                }
                event_loop.timeout_ms(MIO_QUEUE_TIMER, self.opts.queue_timeout as u64 * 1000)
                    .unwrap();

            }

            _ => panic!("Invalid timer id"),

        }
    }


    fn notify(&mut self, _: &mut EventLoop<Self>, notification_type: MioNotification) {
        match self.state {
            MqttState::Connected => {
                match notification_type {
                    MioNotification::Pub(p) => {
                        match p {
                            PubNotify::QoS0 => {
                                let message = {
                                    match self.pub0_rx {
                                        Some(ref pub0_rx) => pub0_rx.recv().unwrap(),
                                        None => panic!("No publish recv channel"),
                                    }
                                };
                                let _ = self._publish(message);
                            }
                            PubNotify::QoS1 |
                            PubNotify::QoS1QueueDown |
                            PubNotify::QoS1Reconnect => {
                                // Increment only if notificication is from publisher
                                if p == PubNotify::QoS1 {
                                    self.pub1_channel_pending += 1;
                                }

                                // Receive from publish channel only when outgoing pub queue
                                // length is < max
                                if self.should_qos1_block == false {
                                    loop {
                                        debug!("Channel pending @@@@@ {}", self.pub1_channel_pending);
                                        // Before
                                        if self.pub1_channel_pending == 0 {
                                            debug!("Finished everything in channel");
                                            break;
                                        }
                                        let mut message = {

                                            match self.pub1_rx {
                                                // Careful, this is a blocking call. Might
                                                // be easier to find queue len bugs with this.
                                                Some(ref pub1_rx) => pub1_rx.recv().unwrap(),
                                                None => panic!("No publish recv channel"),
                                            }
                                        };
                                        // Add next packet id to message and publish
                                        let PacketIdentifier(pkid) = self._next_pkid();
                                        message.set_pkid(pkid);
                                        let _ = self._publish(message);
                                        self.pub1_channel_pending -= 1;
                                    }
                                }
                            }

                            PubNotify::QoS2 |
                            PubNotify::QoS2QueueDown |
                            PubNotify::QoS2Reconnect => {
                                // Increment only if notificication is from publisher
                                if p == PubNotify::QoS2 {
                                    self.pub2_channel_pending += 1;
                                }

                                // Receive from publish channel only when outgoing pub queue
                                // length is < max
                                if self.should_qos2_block == false {
                                    loop {
                                        debug!("QoS2 Channel pending @@@@@ {}", self.pub2_channel_pending);
                                        // Before
                                        if self.pub2_channel_pending == 0 {
                                            debug!("Finished everything in channel");
                                            break;
                                        }
                                        let mut message = {

                                            match self.pub2_rx {
                                                // Careful, this is a blocking call. Might
                                                // be easier to find queue len bugs with this.
                                                Some(ref pub2_rx) => pub2_rx.recv().unwrap(),
                                                None => panic!("No publish recv channel"),
                                            }
                                        };
                                        // Add next packet id to message and publish
                                        let PacketIdentifier(pkid) = self._next_pkid();
                                        message.set_pkid(pkid);
                                        let _ = self._publish(message);
                                        self.pub2_channel_pending -= 1;
                                    }
                                }
                            }
                        }
                    }
                    MioNotification::Sub => {
                        let topics = match self.sub_rx {
                            Some(ref sub_rx) => sub_rx.recv().unwrap(),
                            None => panic!("Expected a subscribe recieve channel"),
                        };
                        let _ = self._subscribe(topics);
                    }
                    MioNotification::Disconnect => {
                        debug!("{:?}", self.state);
                        match self.state {
                            MqttState::Connected => {
                                let _ = self._disconnect();
                                self.state = MqttState::Disconnected;
                            }
                            _ => debug!("Mqtt connection not established"),
                        }
                    }
                    MioNotification::Shutdown => {
                        let _ = self.stream.shutdown(Shutdown::Both);
                    }
                }
            }

            MqttState::Disconnected | MqttState::Handshake => {
                error!("cannot handle user request in handshake/disconnected state")
            }
        }
    }
}

impl ProxyClient {
    // Note: Setting callback before subscriber & publisher
    // are created ensures that message callbacks are registered
    // before subscription & you don't need to pass callbacks through
    // channels (simplifies code)
    pub fn message_callback<F>(mut self, callback: F) -> Self
        where F: Fn(Message) + Send + Sync + 'static
    {
        self.callback = Some(Arc::new(Box::new(callback)));
        self
    }

    /// Connects to the broker and starts an event loop in a new thread.
    /// Returns `Subscriber` and `Publisher` and handles reqests from them.
    /// Also handles network events, reconnections and retransmissions.
    pub fn start(mut self) -> Result<(Publisher, Subscriber)> {
        let mut event_loop = EventLoop::new().unwrap();
        let mionotify_tx = event_loop.channel();
        self.mionotify_tx = Some(mionotify_tx.clone());

        let (pub0_tx, pub0_rx) = mpsc::sync_channel::<Message>(self.opts.pub_q_len as usize);
        self.pub0_rx = Some(pub0_rx);
        let (pub1_tx, pub1_rx) = mpsc::sync_channel::<Message>(self.opts.pub_q_len as usize);
        self.pub1_rx = Some(pub1_rx);
        let (pub2_tx, pub2_rx) = mpsc::sync_channel::<Message>(self.opts.pub_q_len as usize);
        self.pub2_rx = Some(pub2_rx);

        let (sub_tx, sub_rx) = mpsc::sync_channel::<Vec<(TopicFilter, QualityOfService)>>(self.opts.sub_q_len as usize);
        self.sub_rx = Some(sub_rx);

        // synchronizes tcp connection. why ?
        // start() call should fail if there a problem creating initial tcp
        // connection & mqtt connection. Since connections are happening inside thread,
        // this method should be informed to return error instead of
        // (publisher, subscriber) in case connection fails.
        let (connsync_tx, connsync_rx) = mpsc::sync_channel::<MqttStatus>(1);
        self.connsync_tx = Some(connsync_tx);
        // @ Create 'publisher' and 'subscriber'
        // @ These are the handles using which user interacts with rumqtt.
        let publisher = Publisher {
            pub0_tx: pub0_tx,
            pub1_tx: pub1_tx,
            pub2_tx: pub2_tx,
            mionotify_tx: mionotify_tx.clone(),
            retain: false,
        };

        let subscriber = Subscriber {
            subscribe_tx: sub_tx,
            mionotify_tx: mionotify_tx,
        };

        // This is a non blocking call and almost never fails
        // Whether connection is successfu or not is to found
        // out in eventloop
        let stream = try!(TcpStream::connect(&self.addr));
        let stream = NetworkStream::Tcp(stream);
        self.stream = stream;

        thread::spawn(move || {
            // State machine: Disconnected. Check if 'writable' success
            // to know connection success
            self.state = MqttState::Disconnected;
            event_loop.register(self.stream.get_ref().unwrap(),
                          MIO_CLIENT_STREAM,
                          EventSet::writable(),
                          PollOpt::edge() | PollOpt::oneshot())
                .unwrap();

            event_loop.run(&mut self).unwrap();
        });
        let conn = connsync_rx.recv().expect("Connection sync recv error");
        match conn {
            MqttStatus::Success => Ok((publisher, subscriber)),
            MqttStatus::Failed => Err(Error::ConnectionAbort),
        }
    }

    fn handle_packet(&mut self, packet: &VariablePacket) -> Result<HandlePacket> {
        match self.state {
            MqttState::Handshake => {
                match *packet {
                    VariablePacket::ConnackPacket(ref connack) => {
                        let conn_ret_code = connack.connect_return_code();
                        if conn_ret_code != ConnectReturnCode::ConnectionAccepted {
                            error!("Failed to connect, err {:?}", conn_ret_code);
                            Err(Error::ConnectionRefused(conn_ret_code))
                        } else {
                            self.state = MqttState::Connected;
                            Ok(HandlePacket::ConnAck)
                        }
                    }
                    _ => {
                        error!("received invalid packet in handshake state --> {:?}", packet);
                        Ok(HandlePacket::Invalid)
                    }
                }
            }

            MqttState::Connected => {
                match *packet {
                    VariablePacket::SubackPacket(..) => {
                        // if ack.packet_identifier() != 10
                        // TODO: Maintain a subscribe queue and retry if
                        // subscribes are not successful
                        Ok(HandlePacket::SubAck)
                    }

                    VariablePacket::PingrespPacket(..) => {
                        self.await_ping = false;
                        Ok(HandlePacket::PingResp)
                    }

                    // @ Receives disconnect packet
                    VariablePacket::DisconnectPacket(..) => {
                        // TODO
                        Ok(HandlePacket::Disconnect)
                    }

                    // @ Receives puback packet and verifies it with sub packet id
                    VariablePacket::PubackPacket(ref puback) => {
                        // debug!("*** puback --> {:?}\n @@@ queue --> {:#?}",
                        //        puback,
                        //        self.outgoing_pub);
                        let pkid = puback.packet_identifier();
                        match self.outgoing_pub
                            .iter()
                            .position(|ref x| x.1.get_pkid() == Some(pkid)) {
                            Some(i) => {
                                self.outgoing_pub.remove(i);
                            }
                            None => {
                                error!("Oopssss..unsolicited ack");
                            }
                        };
                        debug!("Pub Q Len After Ack @@@ {:?}", self.outgoing_pub.len());
                        Ok(HandlePacket::PubAck)
                    }

                    // @ Receives publish packet
                    VariablePacket::PublishPacket(ref publ) => {
                        // unsafe {
                        //     N += 1;
                        //     println!("N: {}", N);
                        // }
                        let message = try!(Message::from_pub(publ));
                        self._handle_message(message)
                    }

                    // @ Qos2 message published by client is recorded by broker
                    // @ Remove message from 'outgoing_rec' queue and add pkid to 'outgoing_rel'
                    // @ Send 'pubrel' to broker
                    VariablePacket::PubrecPacket(ref pubrec) => {
                        let pkid = pubrec.packet_identifier();
                        match self.outgoing_rec
                            .iter()
                            .position(|ref x| x.1.get_pkid() == Some(pkid)) {
                            Some(i) => {
                                self.outgoing_rec.remove(i);
                            }
                            None => {
                                error!("Oopssss..unsolicited record");
                            }
                        };

                        try!(self._pubrel(pkid));
                        self.outgoing_rel.push_back((time::get_time().sec, PacketIdentifier(pkid)));
                        Ok(HandlePacket::PubRec)
                    }

                    // @ Broker knows that client has the message
                    // @ release the message stored in 'recorded' queue
                    // @ send 'pubcomp' to sender indicating that message is released
                    // @ if 'pubcomp' packet is lost, broker will send pubrel again
                    // @ for the released message, for which we send dummy 'pubcomp' again
                    VariablePacket::PubrelPacket(ref pubrel) => {
                        let pkid = pubrel.packet_identifier();
                        let message = match self.incoming_rec
                            .iter()
                            .position(|ref x| x.get_pkid() == Some(pkid)) {
                            Some(i) => {
                                if let Some(message) = self.incoming_rec.remove(i) {
                                    Some(message)
                                } else {
                                    None
                                }
                            }
                            None => {
                                error!("Oopssss..unsolicited release. Message might have already been released");
                                None
                            }
                        };
                        try!(self._pubcomp(pkid));

                        if let Some(message) = message {
                            Ok(HandlePacket::Publish(message))
                        } else {
                            Ok(HandlePacket::Invalid)
                        }
                    }

                    // @ Remove this pkid from 'outgoing_rel' queue
                    VariablePacket::PubcompPacket(ref pubcomp) => {
                        let pkid = pubcomp.packet_identifier();
                        match self.outgoing_rel
                            .iter()
                            .position(|ref x| x.1 == PacketIdentifier(pkid)) {
                            Some(pos) => self.outgoing_rel.remove(pos),
                            None => {
                                error!("Oopssss..unsolicited complete");
                                None
                            }
                        };
                        Ok(HandlePacket::PubComp)
                    }

                    VariablePacket::UnsubackPacket(..) => Ok(HandlePacket::UnSubAck),

                    _ => Ok(HandlePacket::Invalid), //TODO: Replace this with panic later
                }
            }

            MqttState::Disconnected => {
                error!("Client in disconnected state. Invalid packet received");
                match self._connect() {
                    // @ Change the state machine to handshake
                    // @ and event loop to readable to read
                    // @ CONACK packet
                    Ok(_) => {
                        self.state = MqttState::Handshake;
                    }

                    _ => {
                        error!("There shouldln't be error here");
                        self.state = MqttState::Disconnected;
                    }
                };
                Ok(HandlePacket::Invalid)
            }
        }
    }

    fn _handle_message(&mut self, message: Box<Message>) -> Result<HandlePacket> {
        debug!("       Publish {:?} {:?} < {:?} bytes",
               message.qos,
               message.topic.to_string(),
               message.payload.len());
        match message.qos {
            QoSWithPacketIdentifier::Level0 => Ok(HandlePacket::Publish(message)),
            QoSWithPacketIdentifier::Level1(pkid) => {
                try!(self._puback(pkid));
                Ok(HandlePacket::Publish(message))
            }

            // @ store the message in 'recorded' queue and send 'pubrec' to broker
            // @ if 'pubrec' is lost, broker will resend the message. so only pushback is pkid is new.
            // @ and resend pubcomp.
            // @ TODO: Analyze broker crash cases for all queues.
            QoSWithPacketIdentifier::Level2(pkid) => {
                match self.incoming_rec
                    .iter()
                    .position(|ref x| x.get_pkid() == Some(pkid)) {
                    Some(i) => {
                        self.incoming_rec[i] = message.clone();
                    }
                    None => {
                        self.incoming_rec.push_back(message.clone());
                    }
                };

                try!(self._pubrec(pkid));
                Ok(HandlePacket::PubRec)
            }
        }
    }

    #[allow(non_snake_case)]
    fn STATE_read_incoming(&mut self, event_loop: &mut EventLoop<Self>) -> Result<VariablePacket> {
        match VariablePacket::decode(&mut self.stream) {
            // @ Decoded packet successfully.
            // @ Retrigger readable in evenloop
            Ok(pk) => {
                event_loop.reregister(self.stream.get_ref().unwrap(),
                                MIO_CLIENT_STREAM,
                                EventSet::readable() | EventSet::writable(),
                                PollOpt::edge() | PollOpt::oneshot())
                    .unwrap();
                Ok(pk)
            }
            // @ Try to make a new Tcp connection.
            // @ Set state machine to Disconnected.
            // @ Make eventloop writable to check connection status
            Err(err) => {
                // maybe size=0 while reading indicating socket
                // close at broker end
                error!("Error in receiving packet {:?}", err);
                self._unbind();

                try!(self._try_reconnect());

                self.state = MqttState::Disconnected;
                event_loop.reregister(self.stream.get_ref().unwrap(),
                                MIO_CLIENT_STREAM,
                                EventSet::readable() | EventSet::writable(),
                                PollOpt::edge() | PollOpt::oneshot())
                    .unwrap();
                Err(Error::Read)
            }
        }
    }

    #[allow(non_snake_case)]
    fn STATE_handle_packet(&mut self, packet: &VariablePacket, event_loop: &mut EventLoop<Self>) {
        if let Ok(p) = self.handle_packet(packet) {
            match p {
                // Mqtt connection established, start the timers
                HandlePacket::ConnAck => {
                    if self.initial_connect == true {
                        match self.connsync_tx {
                            Some(ref connsync_tx) => {
                                connsync_tx.send(MqttStatus::Success).expect("Send failed");
                                self.initial_connect = false;
                            }
                            None => panic!("No connsync channel"),
                        }
                    } else {
                        // Publisher won't stop even when disconnected until channel is full.
                        // This notifies notify() to publish channel pending messages after reconnect.
                        match self.mionotify_tx {
                            Some(ref mionotify_tx) => {
                                mionotify_tx.send(MioNotification::Pub(PubNotify::QoS1Reconnect)).expect("Send failed");
                                mionotify_tx.send(MioNotification::Pub(PubNotify::QoS2Reconnect)).expect("Send failed");
                            }
                            None => panic!("No mionotify channel"),
                        }
                    }
                    event_loop.timeout_ms(MIO_QUEUE_TIMER, self.opts.queue_timeout as u64 * 1000).unwrap();
                    if let Some(keep_alive) = self.opts.keep_alive {
                        event_loop.timeout_ms(MIO_PING_TIMER, keep_alive as u64 * 900).unwrap();
                    }

                }
                HandlePacket::Publish(m) => {
                    if let Some(ref message_callback) = self.callback {
                        let message_callback = message_callback.clone();
                        thread::spawn(move || message_callback(*m));
                    }
                }
                // Sending a dummy notification saying tha queue size has reduced
                HandlePacket::PubAck => {
                    // Don't notify everytime q len is < max. This will always be true initially
                    // leading to dup notify.
                    // Send only for notify() to recover if channel is blocked.
                    // Blocking = true is set during publish if pub q len is more than desired.
                    if self.outgoing_pub.len() < self.opts.pub_q_len as usize && self.should_qos1_block == true {
                        match self.mionotify_tx {
                            Some(ref mionotify_tx) => {
                                self.should_qos1_block = false;
                                mionotify_tx.send(MioNotification::Pub(PubNotify::QoS1QueueDown)).expect("Send failed");
                            }
                            None => panic!("No mionotify channel"),
                        }
                    }
                }
                //TODO: Better read from channel again after PubComp instead of PubRec
                HandlePacket::PubRec => {
                    if self.outgoing_rec.len() < self.opts.pub_q_len as usize && self.should_qos2_block == true {
                        match self.mionotify_tx {
                            Some(ref mionotify_tx) => {
                                self.should_qos2_block = false;
                                mionotify_tx.send(MioNotification::Pub(PubNotify::QoS2QueueDown)).expect("Send failed");
                            }
                            None => panic!("No mionotify channel"),
                        }
                    }
                }
                _ => info!("packet handler says that he doesn't care"),
            }
        } else {
            error!("Error handling the packet");
        }
    }

    #[allow(non_snake_case)]
    fn STATE_try_reconnect(&mut self, event_loop: &mut EventLoop<Self>) {
        let _ = self._try_reconnect();

        self.state = MqttState::Disconnected;
        event_loop.reregister(self.stream.get_ref().unwrap(),
                        MIO_CLIENT_STREAM,
                        EventSet::readable() | EventSet::writable(),
                        PollOpt::edge() | PollOpt::oneshot())
            .unwrap();
    }

    fn _connect(&mut self) -> Result<()> {
        let connect = try!(self._generate_connect_packet());
        try!(self._write_packet(connect));
        self._flush()
    }

    pub fn _disconnect(&mut self) -> Result<()> {
        let disconnect = try!(self._generate_disconnect_packet());
        try!(self._write_packet(disconnect));
        self._flush()
    }


    fn _try_reconnect(&mut self) -> Result<()> {
        match self.opts.reconnect {
            None => panic!("To be implemented"),
            Some(dur) => {
                info!("  Will try Reconnect in {} seconds", dur);
                thread::sleep(Duration::new(dur as u64, 0));
                // Almost never fails
                match TcpStream::connect(&self.addr) {
                    Ok(stream) => self.stream = NetworkStream::Tcp(stream),
                    Err(err) => panic!("Error creating new stream {:?}", err),

                }
                Ok(())
            }
        }
    }

    fn _try_retransmit(&mut self) {
        match self.state {
            MqttState::Connected => {
                let outgoing_pub = self.outgoing_pub.clone(); //TODO: Remove the clone
                let outgoing_rec = self.outgoing_rec.clone(); //TODO: Remove the clone
                let outgoing_rel = self.outgoing_rel.clone(); //TODO: Remove the clone
                let timeout = self.opts.queue_timeout as i64;

                // Republish Qos 1 outgoing publishes
                for e in outgoing_pub.iter().filter(|ref x| time::get_time().sec - x.0 > timeout) {
                    let _ = self._publish(*e.1.clone());
                }

                // Republish QoS 2 outgoing records
                for e in outgoing_rec.iter().filter(|ref x| time::get_time().sec - x.0 > timeout) {
                    let _ = self._publish(*e.1.clone());
                }

                // Resend QoS 2 outgoing release
                for e in outgoing_rel.iter().filter(|ref x| time::get_time().sec - x.0 > timeout) {
                    let PacketIdentifier(pkid) = e.1;
                    let _ = self._pubrel(pkid);
                }
            }

            MqttState::Disconnected | MqttState::Handshake => error!("I won't republish. Client isn't in connected state"),
        }
    }

    fn ping(&mut self) -> Result<()> {
        let ping = try!(self._generate_pingreq_packet());
        self.await_ping = true;
        try!(self._write_packet(ping));
        self._flush()
    }

    fn _unbind(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
        self.await_ping = false;
        self.state = MqttState::Disconnected;
        info!("  Disconnected {:?}", self.opts.client_id);
    }

    fn _subscribe(&mut self, topics: Vec<(TopicFilter, QualityOfService)>) -> Result<()> {
        let subscribe_packet = try!(self._generate_subscribe_packet(topics));
        try!(self._write_packet(subscribe_packet));
        self._flush()
    }

    fn _publish(&mut self, message: Message) -> Result<()> {
        let qos = message.qos;
        let message = message.transform(Some(qos));
        let payload = &*message.payload;
        let retain = message.retain;

        let publish_packet = try!(self._generate_publish_packet(message.topic.clone(), qos.clone(), retain, payload.clone()));

        match message.qos {
            QoSWithPacketIdentifier::Level0 => (),
            QoSWithPacketIdentifier::Level1(_) => {
                self.outgoing_pub.push_back((time::get_time().sec, message.clone()));
                if self.outgoing_pub.len() >= self.opts.pub_q_len as usize {
                    self.should_qos1_block = true;
                }
            }
            QoSWithPacketIdentifier::Level2(_) => {
                self.outgoing_rec.push_back((time::get_time().sec, message.clone()));
                if self.outgoing_rec.len() >= self.opts.pub_q_len as usize {
                    self.should_qos2_block = true;
                }
            }
        }
        debug!("       Publish {:?} {:?} > {} bytes", message.qos, message.topic.to_string(), message.payload.len());

        try!(self._write_packet(publish_packet));
        self._flush()
    }

    fn _puback(&mut self, pkid: u16) -> Result<()> {
        let puback_packet = try!(self._generate_puback_packet(pkid));
        try!(self._write_packet(puback_packet));
        self._flush()
    }

    fn _pubrec(&mut self, pkid: u16) -> Result<()> {
        let pubrec_packet = try!(self._generate_pubrec_packet(pkid));
        try!(self._write_packet(pubrec_packet));
        self._flush()
    }

    fn _pubrel(&mut self, pkid: u16) -> Result<()> {
        let pubrel_packet = try!(self._generate_pubrel_packet(pkid));
        try!(self._write_packet(pubrel_packet));
        self._flush()
    }

    fn _pubcomp(&mut self, pkid: u16) -> Result<()> {
        let puback_packet = try!(self._generate_pubcomp_packet(pkid));
        try!(self._write_packet(puback_packet));
        self._flush()
    }

    fn _flush(&mut self) -> Result<()> {
        try!(self.stream.flush());
        self.last_flush = Instant::now();
        Ok(())
    }

    #[inline]
    fn _write_packet(&mut self, packet: Vec<u8>) -> Result<()> {
        // broker.hivemq.com!("@@@ WRITING PACKET\n{:?}", packet);
        try!(self.stream.write_all(&packet));
        Ok(())
    }

    fn _generate_connect_packet(&self) -> Result<Vec<u8>> {
        let mut connect_packet = ConnectPacket::new("MQTT".to_owned(), self.opts.client_id.clone().unwrap());

        connect_packet.set_clean_session(self.opts.clean_session);

        if let Some(keep_alive) = self.opts.keep_alive {
            connect_packet.set_keep_alive(keep_alive);
        }

        // Converting (String, String) -> (TopicName, String)
        let will = match self.opts.will {
            Some(ref will) => Some((try!(TopicName::new(will.0.clone())), will.1.clone())),
            None => None,
        };

        if will.is_some() {
            connect_packet.set_will(will);
            connect_packet.set_will_qos(self.opts.will_qos as u8);
            connect_packet.set_will_retain(self.opts.will_retain);
        }

        // mqtt-protocol APIs are directly handling None cases.
        connect_packet.set_user_name(self.opts.username.clone());
        connect_packet.set_password(self.opts.password.clone());

        let mut buf = Vec::new();

        try!(connect_packet.encode(&mut buf));
        Ok(buf)
    }

    fn _generate_disconnect_packet(&self) -> Result<Vec<u8>> {
        let disconnect_packet = DisconnectPacket::new();
        let mut buf = Vec::new();

        try!(disconnect_packet.encode(&mut buf));
        Ok(buf)
    }

    fn _generate_pingreq_packet(&self) -> Result<Vec<u8>> {
        let pingreq_packet = PingreqPacket::new();
        let mut buf = Vec::new();

        try!(pingreq_packet.encode(&mut buf));
        Ok(buf)
    }

    fn _generate_subscribe_packet(&self, topics: Vec<(TopicFilter, QualityOfService)>) -> Result<Vec<u8>> {
        let subscribe_packet = SubscribePacket::new(11, topics);
        let mut buf = Vec::new();

        try!(subscribe_packet.encode(&mut buf));
        Ok(buf)
    }

    // TODO: dup flag
    fn _generate_publish_packet(&self,
                                topic: TopicName,
                                qos: QoSWithPacketIdentifier,
                                retain: bool,
                                payload: Vec<u8>)
                                -> Result<Vec<u8>> {
        let mut publish_packet = PublishPacket::new(topic, qos, payload);
        let mut buf = Vec::new();
        publish_packet.set_retain(retain);
        // publish_packet.set_dup(dup);
        try!(publish_packet.encode(&mut buf));
        Ok(buf)
    }

    fn _generate_puback_packet(&self, pkid: u16) -> Result<Vec<u8>> {
        let puback_packet = PubackPacket::new(pkid);
        let mut buf = Vec::new();

        try!(puback_packet.encode(&mut buf));
        Ok(buf)
    }

    fn _generate_pubrec_packet(&self, pkid: u16) -> Result<Vec<u8>> {
        let pubrec_packet = PubrecPacket::new(pkid);
        let mut buf = Vec::new();

        try!(pubrec_packet.encode(&mut buf));
        Ok(buf)
    }

    fn _generate_pubrel_packet(&self, pkid: u16) -> Result<Vec<u8>> {
        let pubrel_packet = PubrelPacket::new(pkid);
        let mut buf = Vec::new();

        try!(pubrel_packet.encode(&mut buf));
        Ok(buf)
    }

    fn _generate_pubcomp_packet(&self, pkid: u16) -> Result<Vec<u8>> {
        let pubcomp_packet = PubcompPacket::new(pkid);
        let mut buf = Vec::new();

        try!(pubcomp_packet.encode(&mut buf));
        Ok(buf)
    }

    #[inline]
    fn _next_pkid(&mut self) -> PacketIdentifier {
        let PacketIdentifier(pkid) = self.last_pkid;
        self.last_pkid = PacketIdentifier(pkid + 1);
        self.last_pkid
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MqttStatus {
    Success,
    Failed,
}

enum HandlePacket {
    ConnAck,
    Publish(Box<Message>),
    PubAck,
    PubRec,
    // PubRel,
    PubComp,
    SubAck,
    UnSubAck,
    PingResp,
    Disconnect,
    Invalid,
}

pub struct Publisher {
    pub0_tx: SyncSender<Message>,
    pub1_tx: SyncSender<Message>,
    pub2_tx: SyncSender<Message>,
    mionotify_tx: Sender<MioNotification>,
    retain: bool,
}

impl Publisher {
    pub fn publish(&self, topic: &str, qos: QualityOfService, payload: Vec<u8>) -> Result<()> {

        let topic = try!(TopicName::new(topic.to_string()));
        let qos_pkid = match qos {
            QualityOfService::Level0 => QoSWithPacketIdentifier::Level0,
            QualityOfService::Level1 => QoSWithPacketIdentifier::Level1(0),
            QualityOfService::Level2 => QoSWithPacketIdentifier::Level2(0),
        };

        let message = Message {
            topic: topic,
            retain: self.retain,
            qos: qos_pkid,
            payload: Arc::new(payload),
        };

        // TODO: Check message sanity here and return error if not
        match qos {
            QualityOfService::Level0 => {
                try!(self.pub0_tx.send(message));
                try!(self.mionotify_tx.send(MioNotification::Pub(PubNotify::QoS0)));
            }
            QualityOfService::Level1 => {
                // Order important coz mioco is level triggered
                try!(self.pub1_tx.send(message));
                try!(self.mionotify_tx.send(MioNotification::Pub(PubNotify::QoS1)));
            }
            QualityOfService::Level2 => {
                try!(self.pub2_tx.send(message));
                try!(self.mionotify_tx.send(MioNotification::Pub(PubNotify::QoS2)));
            }
        };

        Ok(())
    }

    pub fn disconnect(&self) -> Result<()> {
        try!(self.mionotify_tx.send(MioNotification::Disconnect));
        Ok(())
    }

    pub fn shutdown(&self) -> Result<()> {
        try!(self.mionotify_tx.send(MioNotification::Shutdown));
        Ok(())
    }

    pub fn set_retain(&mut self, retain: bool) -> &mut Self {
        self.retain = retain;
        self
    }
}

pub type SendableFn = Box<Fn(Message) + Send + Sync>;
pub struct Subscriber {
    subscribe_tx: SyncSender<Vec<(TopicFilter, QualityOfService)>>,
    mionotify_tx: Sender<MioNotification>,
}

impl Subscriber {
    // TODO Add peek function

    pub fn subscribe(&self, topics: Vec<(&str, QualityOfService)>) -> Result<()> {
        let mut sub_topics = vec![];
        for topic in topics {
            let topic = (try!(TopicFilter::new_checked(topic.0)), topic.1);
            sub_topics.push(topic);
        }

        try!(self.subscribe_tx.send(sub_topics));
        try!(self.mionotify_tx.send(MioNotification::Sub));
        Ok(())
    }
}
