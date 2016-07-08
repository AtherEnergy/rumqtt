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
pub struct ClientOptions {
    keep_alive: Option<u16>,
    clean_session: bool,
    client_id: Option<String>,
    username: Option<String>,
    password: Option<String>,
    reconnect: ReconnectMethod,
    pub_q_len: u16,
    sub_q_len: u16,
    queue_timeout: u16, // wait time for ack beyond which packet(publish/subscribe) will be resent
    ssl: Option<SslContext>,
}

impl Default for ClientOptions {
    fn default() -> Self {
        ClientOptions {
            keep_alive: Some(5),
            clean_session: true,
            client_id: None,
            username: None,
            password: None,
            reconnect: ReconnectMethod::ForeverDisconnect,
            pub_q_len: 50,
            sub_q_len: 5,
            queue_timeout: 60,
            ssl: None,
        }
    }
}


impl ClientOptions {
    pub fn new() -> ClientOptions { ClientOptions { ..Default::default() } }

    pub fn set_keep_alive(&mut self, secs: u16) -> &mut Self {
        self.keep_alive = Some(secs);
        self
    }

    pub fn set_client_id(&mut self, client_id: String) -> &mut Self {
        self.client_id = Some(client_id);
        self
    }

    pub fn set_clean_session(&mut self, clean_session: bool) -> &mut Self {
        self.clean_session = clean_session;
        self
    }


    pub fn generate_client_id(&mut self) -> &mut Self {
        let mut rng = rand::thread_rng();
        let id = rng.gen::<u32>();
        self.client_id = Some(format!("mqttc_{}", id));
        self
    }

    pub fn set_username(&mut self, username: String) -> &mut Self {
        self.username = Some(username);
        self
    }

    pub fn set_password(&mut self, password: String) -> &mut Self {
        self.password = Some(password);
        self
    }

    pub fn set_pub_q_len(&mut self, len: u16) -> &mut Self {
        self.pub_q_len = len;
        self
    }

    pub fn set_sub_q_len(&mut self, len: u16) -> &mut Self {
        self.sub_q_len = len;
        self
    }

    pub fn set_reconnect(&mut self, reconnect: ReconnectMethod) -> &mut Self {
        self.reconnect = reconnect;
        self
    }

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

    // TODO: Change name. no connection happenening
    pub fn connect<A: ToSocketAddrs>(mut self, addr: A) -> Result<ProxyClient> {
        if self.client_id == None {
            self.generate_client_id();
        }

        let addr = Self::lookup_ipv4(addr);

        let proxy = ProxyClient {
            addr: addr,
            stream: NetworkStream::None,
            // State
            last_flush: Instant::now(),
            last_pkid: PacketIdentifier(0),
            await_ping: false,
            state: MqttClientState::Disconnected,
            opts: self,

            // Channels
            pub_recv: None,
            sub_recv: None,
            msg_send: None,

            // Queues
            incoming_rec: VecDeque::new(),
            outgoing_pub: VecDeque::new(),
            outgoing_rec: VecDeque::new(),
            outgoing_rel: VecDeque::new(),
        };

        Ok(proxy)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttClientState {
    Handshake,
    Connected, // 0: No state, 1: ping, 2: subscribe, 3: publish, 4: retry
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectMethod {
    ForeverDisconnect,
    ReconnectAfter(Duration),
}

pub enum MioNotification {
    Pub(QualityOfService),
    Sub,
}

pub struct Publisher {
    pub_send: chan::Sender<Message>,
    notifier: Sender<MioNotification>,
}

impl Publisher {
    pub fn publish(&self, topic: &str, qos: QualityOfService, payload: Vec<u8>) -> Result<()> {

        let topic = TopicName::new(topic.to_string()).unwrap(); //TODO: Remove unwrap here
        let qos_pkid = match qos {
            QualityOfService::Level0 => QoSWithPacketIdentifier::Level0,
            QualityOfService::Level1 => QoSWithPacketIdentifier::Level1(0),
            QualityOfService::Level2 => QoSWithPacketIdentifier::Level2(0),
        };
        let message = Message {
            topic: topic,
            retain: false, // TODO: Verify this
            qos: qos_pkid,
            payload: Arc::new(payload),
        };

        // TODO: Check message sanity here and return error if not
        self.pub_send.send(message);
        try!(self.notifier.send(MioNotification::Pub(qos)));
        Ok(())
    }
}

pub struct Subscriber {
    subscribe_send: chan::Sender<Vec<(TopicFilter, QualityOfService)>>,
    message_recv: chan::Receiver<Message>,
    notifier: Sender<MioNotification>,
}

impl Subscriber {
    pub fn subscribe(&self, topics: Vec<(TopicFilter, QualityOfService)>) -> Result<()> {
        // TODO: Check for topic sanity and return error if not
        self.subscribe_send.send(topics);
        try!(self.notifier.send(MioNotification::Sub));
        Ok(())
    }

    // TODO: Multiple topics, timeout
    pub fn receive(&self) -> Result<Message> {
        let message = self.message_recv.recv().unwrap();
        Ok(message)
    }

    // TODO Add peek function
}

/// Handles commands from Publisher and Subscriber. Saves MQTT
/// state and takes care of retransmissions.
pub struct ProxyClient {
    addr: SocketAddr,
    state: MqttClientState,
    opts: ClientOptions,
    stream: NetworkStream,
    last_flush: Instant,
    last_pkid: PacketIdentifier,
    await_ping: bool,

    // Channels
    sub_recv: Option<chan::Receiver<Vec<(TopicFilter, QualityOfService)>>>,
    msg_send: Option<chan::Sender<Message>>,
    pub_recv: Option<chan::Receiver<Message>>,

    /// Queues. Note: 'record' is qos2 term for 'publish'
    /// For QoS 1. Stores outgoing publishes. Removed after `puback` is
    /// received.
    /// If 'puback' isn't received in `queue_timeout` time, client should
    /// resend these messages
    outgoing_pub: VecDeque<(i64, Box<Message>)>,
    /// For QoS 2. Store for incoming publishes to record. Released after
    /// `pubrel` is received.
    /// Client records these messages and sends `pubrec` to broker.
    /// Broker resends these messages if `pubrec` isn't received (which means
    /// either broker message
    /// is lost or clinet `pubrec` is lost) and client responds with `pubrec`.
    /// Broker also resends `pubrel` till it receives `pubcomp` (which means
    /// either borker `pubrel`
    /// is lost or client's `pubcomp` is lost)
    incoming_rec: VecDeque<Box<Message>>, //
    /// For QoS 2. Store for outgoing publishes. Removed after pubrec is
    /// received.
    /// If 'pubrec' isn't received in 'timout' time, client should publish
    /// these messages again.
    outgoing_rec: VecDeque<(i64, Box<Message>)>,
    /// For Qos2. Store for outgoing `pubrel` packets. Removed after `pubcomp`
    /// is received.
    /// If `pubcomp` is not received in `timeout` time (client's `pubrel` might
    /// have been lost and message
    /// isn't released by the broker or broker's `pubcomp` is lost), client
    /// should resend `pubrel` again.
    outgoing_rel: VecDeque<(i64, PacketIdentifier)>,
}

impl ProxyClient {
    /// Returns `Subscriber` and `Publisher` and handles reqests from them
    /// in a seperate thread.
    ///
    /// ```ignore
    /// let proxy_client =
    /// client_options.connect("localhost:1883").expect("CONNECT ERROR");
    /// let (publisher, subscriber) = match proxy_client.await() {
    ///    Ok(h) => h,
    ///    Err(e) => panic!("Await Error --> {:?}", e),
    /// };
    // TODO: CHange name
    pub fn await(mut self) -> Result<(Publisher, Subscriber)> {
        // @ Create notifiers for users to publish to event loop
        let (notify_send, notify_recv) = mpsc::channel::<MioNotification>();
        let (pub_send, pub_recv) = chan::sync::<Message>(self.opts.pub_q_len as usize);
        let (sub_send, sub_recv) = chan::sync::<Vec<(TopicFilter, QualityOfService)>>(self.opts.sub_q_len as usize);
        let (msg_send, msg_recv) = chan::sync::<Message>(0);

        // @ Create 'publisher' and 'subscriber'
        // @ These are the handles using which user interacts with rumqtt.
        let publisher = Publisher {
            pub_send: pub_send,
            notifier: notify_send.clone(),
        };
        self.pub_recv = Some(pub_recv);

        let subscriber = Subscriber {
            subscribe_send: sub_send,
            message_recv: msg_recv,
            notifier: notify_send.clone(),
        };
        self.msg_send = Some(msg_send);
        self.sub_recv = Some(sub_recv);

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
                self.state = MqttClientState::Disconnected;

                // @ Timer for ping requests and retransmits
                let mut ping_timer = mioco::timer::Timer::new();
                let mut resend_timer = mioco::timer::Timer::new();

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
                            self.state = MqttClientState::Handshake;
                        }
                        _ => panic!("There shouldln't be error here"),
                    }

                    // @ Start the event loop
                    loop {
                        select! (
                            r:self.stream.get_ref().unwrap() => {
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
                                        self.state = MqttClientState::Disconnected;
                                        if let Err(e) = self._try_reconnect() {
                                            error!("No Reconnect try --> {:?}", e);
                                            return Err(Error::NoReconnectTry);
                                        }
                                        else {
                                            continue 'mqtt_connect;
                                        }
                                    }
                                };

                                // @ At this point, there is a connected TCP socket
                                match self.handle_packet(&packet) {
                                    Ok(message) => {
                                        if let Some(m) = message {
                                            match self.msg_send {
                                                Some(ref msg_send) => msg_send.send(*m),
                                                None => panic!("Expected a message send channel"),
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        panic!("error in handling packet. {:?}", err);
                                    }
                                };
                            },

                            r:ping_timer => {
                                match self.state {
                                    MqttClientState::Connected => {
                                        if !self.await_ping {
                                            let _ = self.ping();
                                        } else {
                                            error!("awaiting for previous ping resp");
                                        }
                                    }

                                    // @ Timer stopped. Reconnection will start the timer again
                                    MqttClientState::Disconnected |
                                    MqttClientState::Handshake => {
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
                                    MqttClientState::Connected => {
                                        debug!("^^^ QUEUE RESEND");
                                        self._try_retransmit();
                                    }
                                    MqttClientState::Disconnected
                                    | MqttClientState::Handshake => {
                                        debug!("I won't republish.
                                                Client is in disconnected/handshake state")
                                    }
                                }
                                resend_timer.set_timeout(self.opts.queue_timeout as u64 * 1000);
                            },

                            r:notify_recv => {
                                match notify_recv.recv().unwrap() {
                                    MioNotification::Pub(qos) => {
                                        match qos {
                                            QualityOfService::Level0 => {
                                                let message = {
                                                    match self.pub_recv {
                                                        Some(ref pub_recv) => pub_recv.recv().unwrap(),
                                                        None => panic!("No publish recv channel"),
                                                    }
                                                };
                                                let _ = self._publish(message);
                                            }

                                            QualityOfService::Level1 => {
                                                if self.outgoing_pub.len() < self.opts.pub_q_len as usize{
                                                    let mut message = match self.pub_recv {
                                                        Some(ref pub_recv) => pub_recv.recv().unwrap(),
                                                        None => panic!("No publish recv channel"),
                                                    };
                                                    // Add next packet id to message and publish
                                                    let PacketIdentifier(pkid) = self._next_pkid();
                                                    message.set_pkid(pkid);
                                                    let _ = self._publish(message);
                                                }
                                            }

                                            QualityOfService::Level2 => {
                                                if self.outgoing_rec.len() < self.opts.pub_q_len as usize{
                                                    let mut message = match self.pub_recv {
                                                        Some(ref pub_recv) => pub_recv.recv().unwrap(),
                                                        None => panic!("No publish recv channel"),
                                                    };
                                                    // Add next packet id to message and publish
                                                    let PacketIdentifier(pkid) = self._next_pkid();
                                                    message.set_pkid(pkid);
                                                    let _ = self._publish(message);
                                                }
                                            }
                                        }
                                    }
                                    MioNotification::Sub => {
                                        let topics = match self.sub_recv {
                                            Some(ref sub_recv) => sub_recv.recv(),
                                            None => panic!("Expected a subscribe recieve channel"),
                                        };

                                        if let Some(topics) = topics {
                                            info!("request = {:?}", topics);
                                            let _ = self._subscribe(topics);
                                        }
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
            MqttClientState::Handshake => {
                match *packet {
                    VariablePacket::ConnackPacket(ref connack) => {
                        if connack.connect_return_code() != ConnectReturnCode::ConnectionAccepted {
                            error!("Failed to connect, err {:?}", connack.connect_return_code());
                            Ok(None)
                        } else {
                            self.state = MqttClientState::Connected;
                            Ok(None)
                        }
                    }
                    _ => {
                        error!("received invalid packet in handshake state --> {:?}", packet);
                        Ok(None)
                    }
                }
            }

            MqttClientState::Connected => {
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

            MqttClientState::Disconnected => {
                match self._connect() {
                    // @ Change the state machine to handshake
                    // @ and event loop to readable to read
                    // @ CONACK packet
                    Ok(_) => {
                        self.state = MqttClientState::Handshake;
                    }

                    _ => {
                        error!("There shouldln't be error here");
                        self.state = MqttClientState::Disconnected;
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

    pub fn disconnect(&mut self) -> Result<()> {
        let disconnect = try!(self._generate_disconnect_packet());
        try!(self._write_packet(disconnect));
        self._flush()
    }


    fn _try_reconnect(&mut self) -> Result<()> {
        match self.opts.reconnect {
            ReconnectMethod::ForeverDisconnect => Err(Error::NoReconnectTry),
            ReconnectMethod::ReconnectAfter(dur) => {
                // TODO: Move the loop from here to caller
                loop {
                    info!("  Will try Reconnect in {} seconds", dur.as_secs());
                    thread::sleep(dur);
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
            MqttClientState::Connected => {
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

            MqttClientState::Disconnected |
            MqttClientState::Handshake => error!("I won't republish. Client isn't in connected state"),
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
        self.state = MqttClientState::Disconnected;
        info!("  Disconnected {}", self.opts.client_id.clone().unwrap());
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
        connect_packet.set_keep_alive(self.opts.keep_alive.unwrap());

        let mut buf = Vec::new();
        match connect_packet.encode(&mut buf) {
            Ok(result) => result,
            Err(_) => {
                return Err(Error::MqttEncode);
            }
        };
        Ok(buf)
    }

    fn _generate_disconnect_packet(&self) -> Result<Vec<u8>> {
        let disconnect_packet = DisconnectPacket::new();

        let mut buf = Vec::new();
        match disconnect_packet.encode(&mut buf) {
            Ok(result) => result,
            Err(_) => {
                return Err(Error::MqttEncode);
            }
        };
        Ok(buf)
    }

    fn _generate_pingreq_packet(&self) -> Result<Vec<u8>> {
        let pingreq_packet = PingreqPacket::new();
        let mut buf = Vec::new();

        match pingreq_packet.encode(&mut buf) {
            // TODO: Embed all Mqtt errors to rumqtt errors and
            // use try! here and all generate packets
            Ok(result) => result,
            Err(_) => {
                return Err(Error::MqttEncode);
            }
        };
        Ok(buf)
    }

    fn _generate_subscribe_packet(&self, topics: Vec<(TopicFilter, QualityOfService)>) -> Result<Vec<u8>> {
        let subscribe_packet = SubscribePacket::new(11, topics);
        let mut buf = Vec::new();

        subscribe_packet.encode(&mut buf).unwrap();

        match subscribe_packet.encode(&mut buf) {
            Ok(result) => result,
            Err(_) => {
                return Err(Error::MqttEncode);
            }
        };
        Ok(buf)
    }

    fn _generate_publish_packet(&self, topic: TopicName, qos: QoSWithPacketIdentifier, payload: Vec<u8>) -> Result<Vec<u8>> {
        let publish_packet = PublishPacket::new(topic, qos, payload);
        let mut buf = Vec::new();

        match publish_packet.encode(&mut buf) {
            Ok(result) => result,
            Err(_) => {
                return Err(Error::MqttEncode);
            }
        };
        Ok(buf)
    }

    fn _generate_puback_packet(&self, pkid: u16) -> Result<Vec<u8>> {
        let puback_packet = PubackPacket::new(pkid);
        let mut buf = Vec::new();

        match puback_packet.encode(&mut buf) {
            Ok(result) => result,
            Err(_) => {
                return Err(Error::MqttEncode);
            }
        };
        Ok(buf)
    }

    fn _generate_pubrec_packet(&self, pkid: u16) -> Result<Vec<u8>> {
        let pubrec_packet = PubrecPacket::new(pkid);
        let mut buf = Vec::new();

        match pubrec_packet.encode(&mut buf) {
            Ok(result) => result,
            Err(_) => {
                return Err(Error::MqttEncode);
            }
        };
        Ok(buf)
    }

    fn _generate_pubrel_packet(&self, pkid: u16) -> Result<Vec<u8>> {
        let pubrel_packet = PubrelPacket::new(pkid);
        let mut buf = Vec::new();

        match pubrel_packet.encode(&mut buf) {
            Ok(result) => result,
            Err(_) => {
                return Err(Error::MqttEncode);
            }
        };
        Ok(buf)
    }

    fn _generate_pubcomp_packet(&self, pkid: u16) -> Result<Vec<u8>> {
        let pubcomp_packet = PubcompPacket::new(pkid);
        let mut buf = Vec::new();

        match pubcomp_packet.encode(&mut buf) {
            Ok(result) => result,
            Err(_) => {
                return Err(Error::MqttEncode);
            }
        };
        Ok(buf)
    }

    #[inline]
    fn _next_pkid(&mut self) -> PacketIdentifier {
        let PacketIdentifier(pkid) = self.last_pkid;
        self.last_pkid = PacketIdentifier(pkid + 1);
        self.last_pkid
    }
}
