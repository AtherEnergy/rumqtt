use std::time::{Duration, Instant};
use time;

use rand::{self, Rng};
use std::net::{SocketAddr, ToSocketAddrs, Shutdown};
use error::{Error, Result};
use message::Message;
use std::collections::VecDeque;
use std::io::Write;
use std::str;
use mioco;
use mioco::tcp::TcpStream;
use mqtt::{Encodable, Decodable, QualityOfService, TopicFilter};
use mqtt::packet::*;
use mqtt::control::variable_header::{ConnectReturnCode, PacketIdentifier};
use mqtt::topic_name::TopicName;
use std::sync::Arc;
use std::thread;
use chan;
use tls::{SslContext, NetworkStream};
use mioco::sync::mpsc::{self, Sender};

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

    /// Set `username` for broker to perform client authentivcation
    /// via `username` and `password`
    pub fn set_username(&mut self, username: String) -> &mut Self {
        self.username = Some(username);
        self
    }

    /// Set `password` for broker to perform client authentivation
    /// vis `username` and `password`
    pub fn set_password(&mut self, password: String) -> &mut Self {
        self.password = Some(password);
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
        let addrs = addr.to_socket_addrs().unwrap();
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
    /// ```
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
            opts: self.clone(),

            // Queues
            incoming_rec: VecDeque::new(),
            outgoing_pub: VecDeque::new(),
            outgoing_rec: VecDeque::new(),
            outgoing_rel: VecDeque::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MqttState {
    Handshake,
    Connected,
    Disconnected,
}

enum MioNotification {
    Pub(QualityOfService),
    Sub,
    Callback,
    Disconnect,
    Shutdown,
}

pub struct Publisher {
    pub_send: chan::Sender<Message>,
    notifier: Sender<MioNotification>,
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
            retain: self.retain, // TODO: Verify this
            qos: qos_pkid,
            payload: Arc::new(payload),
        };

        // TODO: Check message sanity here and return error if not
        self.pub_send.send(message);
        try!(self.notifier.send(MioNotification::Pub(qos)));
        Ok(())
    }

    pub fn disconnect(&self) -> Result<()> {
        try!(self.notifier.send(MioNotification::Disconnect));
        Ok(())
    }

    pub fn shutdown(&self) -> Result<()> {
        try!(self.notifier.send(MioNotification::Shutdown));
        Ok(())
    }

    pub fn set_retain(&mut self, retain: bool) -> &mut Self {
        self.retain = retain;
        self
    }
}

type SendableFn = Box<Fn(Message) + Send + Sync>;
pub struct Subscriber {
    subscribe_send: chan::Sender<Vec<(TopicFilter, QualityOfService)>>,
    notifier: Sender<MioNotification>,
    callback_send: chan::Sender<SendableFn>,
}

impl Subscriber {
    // TODO Add peek function

    pub fn subscribe(&self, topics: Vec<(&str, QualityOfService)>) -> Result<()> {
        let mut sub_topics = vec![];
        for topic in topics {
            let topic = (try!(TopicFilter::new_checked(topic.0)), topic.1);
            sub_topics.push(topic);
        }

        try!(self.notifier.send(MioNotification::Sub));
        self.subscribe_send.send(sub_topics);
        Ok(())
    }

    pub fn message_callback<F>(&self, callback: F) -> Result<()>
        where F: Fn(Message) + Send + Sync + 'static
    {
        try!(self.notifier.send(MioNotification::Callback));
        self.callback_send.send(Box::new(callback));
        Ok(())
    }
}

/// Handles commands from Publisher and Subscriber. Saves MQTT
/// state and takes care of retransmissions.
pub struct ProxyClient {
    addr: SocketAddr,
    state: MqttState,
    opts: MqttOptions,
    stream: NetworkStream,
    last_flush: Instant,
    last_pkid: PacketIdentifier,
    await_ping: bool,

    /// Queues. Note: 'record' is qos2 term for 'publish'
    /// For QoS 1. Stores outgoing publishes
    outgoing_pub: VecDeque<(i64, Box<Message>)>,
    /// For QoS 2. Store for incoming publishes to record.
    incoming_rec: VecDeque<Box<Message>>, //
    /// For QoS 2. Store for outgoing publishes.
    outgoing_rec: VecDeque<(i64, Box<Message>)>,
    /// For Qos2. Store for outgoing `pubrel` packets.
    outgoing_rel: VecDeque<(i64, PacketIdentifier)>,
}

impl ProxyClient {
    /// Connects to the broker and starts an event loop in a new thread.
    /// Returns `Subscriber` and `Publisher` and handles reqests from them.
    /// Also handles network events, reconnections and retransmissions.
    pub fn start(mut self) -> Result<(Publisher, Subscriber)> {
        // @ Create notifiers for users to publish to event loop
        let (notify_send, notify_recv) = mpsc::channel::<MioNotification>();
        let (pub_send, pub_recv) = chan::sync::<Message>(self.opts.pub_q_len as usize);
        let (sub_send, sub_recv) = chan::sync::<Vec<(TopicFilter, QualityOfService)>>(self.opts.sub_q_len as usize);
        let (callback_send, callback_recv) = chan::sync::<SendableFn>(0);

        // @ Create 'publisher' and 'subscriber'
        // @ These are the handles using which user interacts with rumqtt.
        let publisher = Publisher {
            pub_send: pub_send,
            notifier: notify_send.clone(),
            retain: false,
        };

        let subscriber = Subscriber {
            subscribe_send: sub_send,
            notifier: notify_send.clone(),
            callback_send: callback_send,
        };

        // @ New thread for event loop
        thread::spawn(move || -> Result<()> {
            mioco::start(move || -> Result<()> {
                // @ Inital Tcp/Tls connection.
                // @ Won't retry if connectin fails here
                let stream: TcpStream = try!(TcpStream::connect(&self.addr));
                let stream = match self.opts.ssl {
                    Some(ref ssl) => NetworkStream::Ssl(try!(ssl.connect(stream))),
                    None => NetworkStream::Tcp(stream),
                };
                self.stream = stream;
                self.state = MqttState::Disconnected;

                // @ Timer for ping requests and retransmits
                let mut ping_timer = mioco::timer::Timer::new();
                let mut resend_timer = mioco::timer::Timer::new();

                // On Message Callback
                let mut eloop_callback: Option<Arc<SendableFn>> = None;

                'mqtt_connect: loop {
                    // @ Send Mqtt connect packet.
                    // @ Change the state to handshake.
                    // @ Start PINGREQ/RETRANSMIT timer. But PINGREQs
                    // @ should only be sent in 'connected' state
                    match self._connect() {
                        Ok(_) => {
                            if let Some(keep_alive) = self.opts.keep_alive {
                                ping_timer.set_timeout(keep_alive as u64 * 900);
                                resend_timer.set_timeout(self.opts.queue_timeout as u64 * 1000);
                            }
                            self.state = MqttState::Handshake;
                        }
                        _ => panic!("There shouldln't be error here"),
                    }

                    // @ Start the event loop
                    loop {
                        select! (
                            r:try!(self.stream.get_ref()) => {
                                let packet = match VariablePacket::decode(&mut self.stream) {
                        // @ Decoded packet successfully.
                                    Ok(pk) => pk,
                        // @ Set state machine to Disconnected.
                        // @ Try to make a new Tcp connection.
                        // @ If network goes down after this block, all the network operations ..
                        // @ will fail until eventloop reaches here again and tries for reconnect.
                                    Err(err) => {
                        // maybe size=0 while reading indicating socket
                        // close at broker end
                                        error!("Error in receiving packet {:?}", err);
                                        self._unbind();
                                        self.state = MqttState::Disconnected;
                                        if let Err(e) = self._try_reconnect() {
                                            error!("No Reconnect try --> {:?}", e);
                                            return Err(Error::NoReconnectTry);
                                        }
                                        else {
                                            continue 'mqtt_connect;
                                        }
                                    }
                                };
                                trace!("{:?}", packet);
                                // @ At this point, there is a connected TCP socket
                                // @ Exits the event loop with error if handshake fails
                                let message =  try!(self.handle_packet(&packet));
                                if let Some(m) = message {
                                    if let Some(ref eloop_callback) = eloop_callback {
                                        let eloop_callback = eloop_callback.clone();
                                        mioco::spawn(move || eloop_callback(*m));
                                    }
                                } 
                            },

                            r:ping_timer => {
                                match self.state {
                                    MqttState::Connected => {
                                        if !self.await_ping {
                                            let _ = self.ping();
                                        } else {
                                            error!("awaiting for previous ping resp");
                                        }
                                    }

                                    // @ Timer stopped. Reconnection will start the timer again
                                    MqttState::Disconnected |
                                    MqttState::Handshake => {
                                        debug!("I won't ping.
                                                Client is in disconnected/handshake state")
                                    }
                                }
                                // @ Restarting the timer only in connected state
                                // @ is leading to level triggered notifications.
                                // TODO: Find a way to stop the timer in
                                // Disconnected/Handshake state
                                if let Some(keep_alive) = self.opts.keep_alive {
                                    ping_timer.set_timeout(keep_alive as u64 * 900);
                                }
                            },

                            r:resend_timer => {
                                match self.state {
                                    MqttState::Connected => {
                                        debug!("^^^ QUEUE RESEND");
                                        self._try_retransmit();
                                    }
                                    MqttState::Disconnected
                                    | MqttState::Handshake => {
                                        debug!("I won't republish.
                                                Client is in disconnected/handshake state")
                                    }
                                }
                                resend_timer.set_timeout(self.opts.queue_timeout as u64 * 1000);
                            },

                            r:notify_recv => {
                                match try!(notify_recv.recv()) {
                                    MioNotification::Pub(qos) => {
                                        match qos {
                                            QualityOfService::Level0 => {
                                                let message = pub_recv.recv().unwrap();
                                                let _ = self._publish(message);
                                            }

                                            QualityOfService::Level1 => {
                                                if self.outgoing_pub.len() < self.opts.pub_q_len as usize{
                                                    let mut message = pub_recv.recv().unwrap();
                                                    // Add next packet id to message and publish
                                                    let PacketIdentifier(pkid) = self._next_pkid();
                                                    message.set_pkid(pkid);
                                                    let _ = self._publish(message);
                                                }
                                            }

                                            QualityOfService::Level2 => {
                                                if self.outgoing_rec.len() < self.opts.pub_q_len as usize{
                                                    let mut message = pub_recv.recv().unwrap();
                                                    // Add next packet id to message and publish
                                                    let PacketIdentifier(pkid) = self._next_pkid();
                                                    message.set_pkid(pkid);
                                                    let _ = self._publish(message);
                                                }
                                            }
                                        }
                                    }
                                    MioNotification::Sub => {
                                        let topics = sub_recv.recv().unwrap();
                                        let _ = self._subscribe(topics.clone());
                                    }

                                    MioNotification::Disconnect => {
                                         match self.state {
                                            MqttState::Connected => {
                                                let _ = self._disconnect();
                                            }
                                            _ => debug!("Mqtt connection not established"),
                                        }
                                    }

                                    MioNotification::Shutdown => {
                                        let _ = self.stream.shutdown(Shutdown::Both);
                                    }

                                    MioNotification::Callback => {
                                        let callback = callback_recv.recv().expect("Expected a callback");
                                        // Set eventloop callback
                                        eloop_callback = Some(Arc::new(callback));
                                    }
                                }
                            },

                        ); //select end
                    } //event loop end
                } //mqtt connection loop end

            }); //mioco end
            Err(Error::EventLoop)
        }); //thread end

        Ok((publisher, subscriber))
    }

    fn handle_packet(&mut self, packet: &VariablePacket) -> Result<Option<Box<Message>>> {
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
                            Ok(None)
                        }
                    }
                    _ => {
                        error!("received invalid packet in handshake state --> {:?}", packet);
                        Ok(None)
                    }
                }
            }

            MqttState::Connected => {
                match *packet {
                    VariablePacket::SubackPacket(..) => {
                        // if ack.packet_identifier() != 10
                        // TODO: Maintain a subscribe queue and retry if
                        // subscribes are not successful
                        Ok(None)
                    }

                    VariablePacket::PingrespPacket(..) => {
                        self.await_ping = false;
                        Ok(None)
                    }

                    // @ Receives disconnect packet
                    VariablePacket::DisconnectPacket(..) => {
                        // TODO
                        Ok(None)
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
                        Ok(None)
                    }

                    // @ Receives publish packet
                    VariablePacket::PublishPacket(ref publ) => {
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
                                self.outgoing_pub.remove(i);
                            }
                            None => {
                                error!("Oopssss..unsolicited record");
                            }
                        };

                        try!(self._pubrel(pkid));
                        self.outgoing_rel.push_back((time::get_time().sec, PacketIdentifier(pkid)));
                        Ok(None)
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
                                info!("Oopssss..unsolicited release. Message might have already been released");
                                None
                            }
                        };
                        try!(self._pubcomp(pkid));
                        Ok(message)
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
                        Ok(None)
                    }

                    VariablePacket::UnsubackPacket(..) => Ok(None),

                    _ => Ok(None), //TODO: Replace this with panic later
                }
            }

            MqttState::Disconnected => {
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
                Ok(None)
            }
        }
    }

    fn _handle_message(&mut self, message: Box<Message>) -> Result<Option<Box<Message>>> {
        debug!("       Publish {:?} {:?} < {:?} bytes",
               message.qos,
               message.topic.to_string(),

               message.payload.len());
        match message.qos {
            QoSWithPacketIdentifier::Level0 => Ok(Some(message)),
            QoSWithPacketIdentifier::Level1(pkid) => {
                try!(self._puback(pkid));
                Ok(Some(message))
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
                Ok(None)
            }
        }
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
            None => Err(Error::NoReconnectTry),
            Some(dur) => {
                // TODO: Move the loop from here to caller
                loop {
                    info!("  Will try Reconnect in {} seconds", dur);
                    thread::sleep(Duration::new(dur as u64, 0));
                    match TcpStream::connect(&self.addr) {
                        Ok(stream) => {
                            let stream = match self.opts.ssl {
                                Some(ref ssl) => NetworkStream::Ssl(try!(ssl.connect(stream))),
                                None => NetworkStream::Tcp(stream),
                            };
                            self.stream = stream;
                            break;
                        }
                        Err(err) => {
                            error!("Error creating new stream {:?}", err);
                            continue;
                        }
                    }
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

        let publish_packet = try!(self._generate_publish_packet(message.topic.clone(), qos.clone(), payload.clone()));

        match message.qos {
            QoSWithPacketIdentifier::Level0 => (),
            QoSWithPacketIdentifier::Level1(_) => self.outgoing_pub.push_back((time::get_time().sec, message.clone())),
            QoSWithPacketIdentifier::Level2(_) => self.outgoing_rec.push_back((time::get_time().sec, message.clone())),
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
        // trace!("@@@ WRITING PACKET\n{:?}", packet);
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

    fn _generate_publish_packet(&self, topic: TopicName, qos: QoSWithPacketIdentifier, payload: Vec<u8>) -> Result<Vec<u8>> {
        let publish_packet = PublishPacket::new(topic, qos, payload);
        let mut buf = Vec::new();

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
