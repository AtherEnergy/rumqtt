use std::time::{Duration, Instant};
use rand::{self, Rng};
use std::net::{SocketAddr, ToSocketAddrs};
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
use chan;
use std::net::Shutdown;

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
}


impl ClientOptions {
    pub fn new() -> ClientOptions {
        ClientOptions {
            keep_alive: Some(5),
            clean_session: true,
            client_id: None,
            username: None,
            password: None,
            reconnect: ReconnectMethod::ForeverDisconnect,
            pub_q_len: 50,
            sub_q_len: 5,
        }
    }

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

    pub fn set_reconnect(&mut self, reconnect: ReconnectMethod) -> &mut ClientOptions {
        self.reconnect = reconnect;
        self
    }

    pub fn connect<A: ToSocketAddrs>(mut self, addr: A) -> Result<ProxyClient> {
        if self.client_id == None {
            self.generate_client_id();
        }

        let addr = try!(addr.to_socket_addrs()).next().expect("Socket address is broken");
        let stream = TcpStream::connect(&addr).unwrap();

        let proxy = ProxyClient {
            addr: addr,
            stream: stream,
            session_present: false,
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
            incomming_pub: VecDeque::new(),
            incomming_rec: VecDeque::new(),
            incomming_rel: VecDeque::new(),
            outgoing_ack: VecDeque::new(),
            outgoing_rec: VecDeque::new(),
            outgoing_comp: VecDeque::new(),
        };

        Ok(proxy)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttClientState {
    Handshake,
    Connected,
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
    mio_notifier: Sender<MioNotification>,
}

impl Publisher {
    pub fn publish(&self, topic: &str, qos: QualityOfService, payload: Vec<u8>) {

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

        self.pub_send.send(message);
        self.mio_notifier.send(MioNotification::Pub(qos));
    }
}

pub struct Subscriber {
    subscribe_send: chan::Sender<Vec<(TopicFilter, QualityOfService)>>,
    message_recv: chan::Receiver<Message>,
    mio_notifier: Sender<MioNotification>,
}

impl Subscriber {
    pub fn subscribe(&self, topics: Vec<(TopicFilter, QualityOfService)>) {
        self.subscribe_send.send(topics);
        self.mio_notifier.send(MioNotification::Sub);
    }

    pub fn receive(&self) -> Result<Message> {
        let message = self.message_recv.recv().unwrap();
        Ok(message)
    }
}


pub struct ProxyClient {
    addr: SocketAddr,
    state: MqttClientState,
    opts: ClientOptions,
    stream: TcpStream,
    session_present: bool,
    last_flush: Instant,
    last_pkid: PacketIdentifier,
    await_ping: bool,

    // Channels
    sub_recv: Option<chan::Receiver<Vec<(TopicFilter, QualityOfService)>>>,
    msg_send: Option<chan::Sender<Message>>,
    pub_recv: Option<chan::Receiver<Message>>,

    // Queues
    incomming_pub: VecDeque<Box<Message>>, // QoS 1
    incomming_rec: VecDeque<Box<Message>>, // QoS 2
    incomming_rel: VecDeque<PacketIdentifier>, // QoS 2
    outgoing_ack: VecDeque<Box<Message>>, // QoS 1
    outgoing_rec: VecDeque<Box<Message>>, // QoS 2
    outgoing_comp: VecDeque<PacketIdentifier>, // QoS 2
}

impl Handler for ProxyClient {
    type Timeout = u64;
    type Message = MioNotification;

    fn ready(&mut self, event_loop: &mut EventLoop<ProxyClient>, token: Token, _: EventSet) {
        match token {
            Token(1) => {
                let packet = match VariablePacket::decode(&mut self.stream) {
                    Ok(pk) => pk,
                    Err(err) => {
                        // maybe size=0 while reading indicating socket
                        // close at broker end
                        error!("Error in receiving packet {:?}", err);
                        self._unbind();

                        if let Err(e) = self._try_reconnect() {
                            error!("No Reconnect try --> {:?}", e);
                            return;
                        }

                        event_loop.reregister(&self.stream,
                                              Token(1),
                                              EventSet::readable(),
                                              PollOpt::edge());

                        // Mqtt connect packet send
                        match self._connect() {
                            Ok(_) => {
                                if let Some(keep_alive) = self.opts.keep_alive {
                                    event_loop.timeout_ms(123, keep_alive as u64 * 900);
                                }
                            }
                            Err(e) => debug!("Error Reconnecting --> {:?}", e),
                        }
                        return;

                    }
                };

                trace!("   %%%RECEIVED PACKET {:?}", packet);
                match self.handle_packet(&packet) {
                    Ok(message) => {
                        if let Some(m) = message {
                            match self.msg_send {
                                Some(ref msg_send) => msg_send.send(*m),
                                None => panic!("Expected a message send channel"),
                            }
                        }
                    }
                    Err(err) => panic!("error in handling packet. {:?}", err),
                };


            }
            _ => panic!("unexpected token"),
        }
    }

    fn timeout(&mut self, event_loop: &mut EventLoop<Self>, timeout: Self::Timeout) {

        println!("client state --> {:?}, await_ping --> {}",
                 self.state,
                 self.await_ping);

        match self.state {
            MqttClientState::Connected => {
                if !self.await_ping {
                    let _ = self.ping();
                } else {
                    panic!("awaiting for previous ping resp");
                }

                if let Some(keep_alive) = self.opts.keep_alive {
                    event_loop.timeout_ms(123, keep_alive as u64 * 900);
                }
            }

            MqttClientState::Disconnected |
            MqttClientState::Handshake => {
                debug!("I won't ping. Client is in disconnected/handshake state")
            }

        }
    }

    fn notify(&mut self, event_loop: &mut EventLoop<Self>, notification_type: MioNotification) {
        match notification_type {
            MioNotification::Pub(qos) => {
                match qos {
                    QualityOfService::Level0 => {
                        let message = {
                            match self.pub_recv {
                                Some(ref pub_recv) => pub_recv.recv().unwrap(),
                                None => panic!("No publish recv channel"),
                            }
                        };
                        self._publish(message);
                    }
                    QualityOfService::Level1 => {
                        if self.outgoing_ack.len() < 5 {
                            let mut message = {
                                match self.pub_recv {
                                    Some(ref pub_recv) => pub_recv.recv().unwrap(),
                                    None => panic!("No publish recv channel"),
                                }
                            };

                            // Add next packet id to message and publish
                            let PacketIdentifier(pkid) = self._next_pkid();
                            message.set_pkid(pkid);
                            self._publish(message);
                        }
                    }

                    _ => panic!("Invalid Qos"),
                }
            }
            MioNotification::Sub => {
                let topics = match self.sub_recv {
                    Some(ref sub_recv) => sub_recv.recv(),
                    None => panic!("Expected a subscribe recieve channel"),
                };

                if let Some(topics) = topics {
                    info!("request = {:?}", topics);
                    self._subscribe(topics);
                }
            }
        }

    }
}

impl ProxyClient {
    pub fn await(mut self) -> Result<(Publisher, Subscriber)> {
        try!(self._connect());


        let mut event_loop = EventLoop::new().unwrap();
        let mio_notify = event_loop.channel();

        let (pub_send, pub_recv) = chan::sync::<Message>(self.opts.pub_q_len as usize);
        let publisher = Publisher {
            pub_send: pub_send,
            mio_notifier: mio_notify.clone(),
        };
        self.pub_recv = Some(pub_recv);

        let (sub_send, sub_recv) = chan::sync::<Vec<(TopicFilter, QualityOfService)>>(self.opts.sub_q_len as usize);
        let (msg_send, msg_recv) = chan::sync::<Message>(0);
        let subscriber = Subscriber {
            subscribe_send: sub_send,
            message_recv: msg_recv,
            mio_notifier: mio_notify.clone(),
        };
        self.msg_send = Some(msg_send);
        self.sub_recv = Some(sub_recv);

        thread::spawn(move || {
            event_loop.register(&self.stream,
                                Token(1),
                                EventSet::readable(),
                                PollOpt::edge())
                      .unwrap();

            if let Some(keep_alive) = self.opts.keep_alive {
                event_loop.timeout_ms(123, keep_alive as u64 * 900);
            }
            event_loop.run(&mut self).unwrap();
        });
        Ok((publisher, subscriber))
    }

    fn handle_packet(&mut self, packet: &VariablePacket) -> Result<Option<Box<Message>>> {
        match self.state {
            MqttClientState::Handshake => {
                match packet {
                    &VariablePacket::ConnackPacket(ref connack) => {
                        if connack.connect_return_code() != ConnectReturnCode::ConnectionAccepted {

                            error!("Failed to connect, err {:?}", connack.connect_return_code());

                            match self._try_reconnect() {
                                Ok(_) => Ok(None), //sent connect packet
                                Err(e) => Err(Error::NoReconnectTry), //no conn packet sent
                            }
                        } else {
                            self.state = MqttClientState::Connected;
                            Ok(None)
                        }
                    }
                    _ => Ok(None),
                }
            }

            MqttClientState::Connected => {
                match packet {
                    &VariablePacket::SubackPacket(..) => {
                        // if ack.packet_identifier() != 10
                        // TODO: Maintain a subscribe queue and retry if
                        // subscribes are not successful
                        Ok(None)
                    }

                    &VariablePacket::PingrespPacket(..) => {
                        self.await_ping = false;
                        Ok(None)
                    }

                    /// Receives disconnect packet
                    &VariablePacket::DisconnectPacket(..) => {
                        // TODO
                        Ok(None)
                    }

                    /// Receives puback packet and verifies it with sub packet id
                    &VariablePacket::PubackPacket(ref puback) => {
                        debug!("*** puback --> {:?}\n @@@ queue --> {:#?}",
                               puback,
                               self.outgoing_ack);
                        let pkid = puback.packet_identifier();
                        match self.outgoing_ack
                                  .iter()
                                  .position(|ref x| x.get_pkid() == Some(pkid)) {
                            Some(i) => self.outgoing_ack.remove(i),
                            None => panic!("Oopssss..unsolicited ack"),
                        };
                        Ok(None)
                    }

                    /// Receives publish packet
                    &VariablePacket::PublishPacket(ref publ) => {
                        let message = try!(Message::from_pub(publ));
                        self._handle_message(message)
                    }

                    &VariablePacket::PubrecPacket(..) => Ok(None),

                    &VariablePacket::PubrelPacket(..) => Ok(None),

                    &VariablePacket::PubcompPacket(..) => Ok(None),

                    &VariablePacket::UnsubackPacket(..) => Ok(None),

                    _ => Ok(None), //TODO: Replace this with panic later
                }
            }

            MqttClientState::Disconnected => Err(Error::ConnectionAbort),
        }
    }

    fn _handle_message(&mut self, message: Box<Message>) -> Result<Option<Box<Message>>> {
        debug!("       Publish {:?} {:?} < {:?} bytes",
               message.qos,
               message.topic.to_string(),
               message.payload.len());
        match message.qos {
            QoSWithPacketIdentifier::Level0 => Ok(Some(message)),
            QoSWithPacketIdentifier::Level1(_) => Ok(Some(message)),
            QoSWithPacketIdentifier::Level2(_) => Ok(None),
        }
    }

    fn _connect(&mut self) -> Result<()> {
        let connect = try!(self._generate_connect_packet());
        try!(self._write_packet(connect));
        self.state = MqttClientState::Handshake;
        self._flush()
    }

    fn _try_reconnect(&mut self) -> Result<()> {
        match self.opts.reconnect {
            ReconnectMethod::ForeverDisconnect => Err(Error::NoReconnectTry),
            ReconnectMethod::ReconnectAfter(dur) => {
                info!("  Will try Reconnect in {} seconds", dur.as_secs());
                thread::sleep(dur);
                match TcpStream::connect(&self.addr) {
                    Ok(stream) => {
                        println!("stream = {:?}", stream);
                        self.stream = stream;
                    }
                    Err(err) => {
                        panic!("Error creating new stream {:?}", err);
                    }
                }
                Ok(())
            }
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

        let qos = message.qos.clone();
        let message = message.transform(Some(qos.clone()));
        let ref payload = *message.payload;

        let publish_packet = try!(self._generate_publish_packet(message.topic.clone(),
                                                                qos.clone(),
                                                                payload.clone()));

        match message.qos {
            QoSWithPacketIdentifier::Level0 => (),
            QoSWithPacketIdentifier::Level1(_) => self.outgoing_ack.push_back(message.clone()),
            QoSWithPacketIdentifier::Level2(_) => (),
        }

        debug!("       Publish {:?} {:?} > {} bytes",
               message.qos,
               message.topic.to_string(),
               message.payload.len());

        try!(self._write_packet(publish_packet));
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
        let mut connect_packet = ConnectPacket::new("MQTT".to_owned(),
                                                    self.opts.client_id.clone().unwrap());
        connect_packet.set_clean_session(self.opts.clean_session);
        connect_packet.set_keep_alive(self.opts.keep_alive.unwrap());

        let mut buf = Vec::new();
        match connect_packet.encode(&mut buf) {
            Ok(result) => result,
            Err(_) => {
                return Err(Error::MqttEncodeError);
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
                return Err(Error::MqttEncodeError);
            }
        };
        Ok(buf)
    }

    fn _generate_subscribe_packet(&self,
                                  topics: Vec<(TopicFilter, QualityOfService)>)
                                  -> Result<Vec<u8>> {
        let subscribe_packet = SubscribePacket::new(11, topics);
        let mut buf = Vec::new();

        subscribe_packet.encode(&mut buf).unwrap();

        match subscribe_packet.encode(&mut buf) {
            Ok(result) => result,
            Err(_) => {
                return Err(Error::MqttEncodeError);
            }
        };
        Ok(buf)
    }

    fn _generate_publish_packet(&self,
                                topic: TopicName,
                                qos: QoSWithPacketIdentifier,
                                payload: Vec<u8>)
                                -> Result<Vec<u8>> {
        let publish_packet = PublishPacket::new(topic, qos, payload);
        let mut buf = Vec::new();

        match publish_packet.encode(&mut buf) {
            Ok(result) => result,
            Err(_) => {
                return Err(Error::MqttEncodeError);
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