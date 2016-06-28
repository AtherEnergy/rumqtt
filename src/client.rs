use std::time::{Duration, Instant};
use rand::{self, Rng};
use std::net::{SocketAddr, ToSocketAddrs};
use error::{Error, Result};
use message::Message;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::str;
use mio::tcp::TcpStream;
use mio::*;
use mqtt::{Encodable, Decodable, QualityOfService, TopicFilter};
use mqtt::packet::*;
use mqtt::control::variable_header::{ConnectReturnCode, PacketIdentifier};
use mqtt::topic_name::TopicName;
use std::sync::{self, Arc, Mutex};
use std::sync::mpsc;
use std::io::Cursor;
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
        }
    }

    pub fn set_keep_alive(&mut self, secs: u16) -> &mut ClientOptions {
        self.keep_alive = Some(secs);
        self
    }

    pub fn set_client_id(&mut self, client_id: String) -> &mut ClientOptions {
        self.client_id = Some(client_id);
        self
    }

    pub fn set_clean_session(&mut self, clean_session: bool) -> &mut ClientOptions {
        self.clean_session = clean_session;
        self
    }


    pub fn generate_client_id(&mut self) -> &mut ClientOptions {
        let mut rng = rand::thread_rng();
        let id = rng.gen::<u32>();
        self.client_id = Some(format!("mqttc_{}", id));
        self
    }

    pub fn set_username(&mut self, username: String) -> &mut ClientOptions {
        self.username = Some(username);
        self
    }

    pub fn set_password(&mut self, password: String) -> &mut ClientOptions {
        self.password = Some(password);
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

        // let (sub_send, sub_recv) = mioco::sync::mpsc::channel::<Vec<(TopicFilter,
        //                                                              QualityOfService)>>();
        // let (msg_send, msg_recv) = mioco::sync::mpsc::channel::<Message>();

        let mut proxy = ProxyClient {
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

            // Queues
            incomming_pub: VecDeque::new(),
            incomming_rec: VecDeque::new(),
            incomming_rel: VecDeque::new(),
            outgoing_ack: VecDeque::new(),
            outgoing_rec: VecDeque::new(),
            outgoing_comp: VecDeque::new(),
        };

        // Ok((proxy, subscriber, publisher))
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

pub struct Publisher {
    pub_send: chan::Sender<Message>,
    mio_notifier: Sender<bool>,
}

impl Publisher {
    pub fn publish(&self, topic: &str, qos: QualityOfService, payload: Vec<u8>) {

        let topic = TopicName::new(topic.to_string()).unwrap(); //TODO: Remove unwrap here
        let qos = match qos {
            QualityOfService::Level0 => QoSWithPacketIdentifier::Level0,
            QualityOfService::Level1 => QoSWithPacketIdentifier::Level1(0),
            QualityOfService::Level2 => QoSWithPacketIdentifier::Level2(0),
        };
        let message = Message {
            topic: topic,
            retain: false, // TODO: Verify this
            qos: qos,
            payload: Arc::new(payload),
        };

        self.pub_send.send(message);
        self.mio_notifier.send(true);
    }
}


// pub struct Proxy {
//     addr: SocketAddr,
//     opts: ClientOptions,
//     stream: Option<TcpStream>,
//     session_present: bool,
//     //subscribe_recv: Receiver<Vec<(TopicFilter, QualityOfService)>>,
//     //message_send: Sender<Message>,
//     //pub_recv: mpsc::Receiver<Message>,
// }

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
    type Message = bool;

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
                            // message_send.send(*m);
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

    fn notify(&mut self, event_loop: &mut EventLoop<Self>, msg: bool) {
        if self.outgoing_ack.len() < 5 {
            let message = {
                match self.pub_recv {
                    Some(ref pub_recv) => pub_recv.recv().unwrap(),
                    None => panic!("No publish recv channel"),
                }
            };

            self.outgoing_ack.push_back(Box::new(message.clone()));
            self._publish(message);
        }
    }
}

impl ProxyClient {
    pub fn await(mut self) -> Result<(Publisher, i32)> {
        try!(self._connect());

        // let subscriber = Subscriber {
        //     subscribe_send: sub_send,
        //     message_recv: msg_recv,
        // };
        let mut event_loop = EventLoop::new().unwrap();
        let mio_notify = event_loop.channel();

        let (pub_send, pub_recv) = chan::sync::<Message>(10);
        let publisher = Publisher {
            pub_send: pub_send,
            mio_notifier: mio_notify.clone(),
        };
        self.pub_recv = Some(pub_recv);

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
        Ok((publisher, 0))
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
                    &VariablePacket::SubackPacket(ref ack) => {
                        if ack.packet_identifier() != 10 {
                            error!("SUBACK packet identifier not match");
                        } else {
                            println!("Subscribed!");
                        }

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
                        let pkid = puback.packet_identifier();
                        Ok(None)
                    }

                    /// Receives publish packet
                    &VariablePacket::PublishPacket(ref publ) => {
                        let message = try!(Message::from_pub(publ));
                        self._handle_message(message)
                    }

                    &VariablePacket::PubrecPacket(ref pubrec) => Ok(None),

                    &VariablePacket::PubrelPacket(ref pubrel) => Ok(None),

                    &VariablePacket::PubcompPacket(ref pubcomp) => Ok(None),

                    &VariablePacket::UnsubackPacket(ref pubrec) => Ok(None),

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
        // TODO: sync wait for suback here
    }

    fn _publish(&mut self, message: Message) -> Result<()> {

        let qos = match message.qos.clone() {
            QoSWithPacketIdentifier::Level0 => QoSWithPacketIdentifier::Level0,
            QoSWithPacketIdentifier::Level1(_) |
            QoSWithPacketIdentifier::Level2(_) => {
                let PacketIdentifier(next_pkid) = self._next_pkid();
                QoSWithPacketIdentifier::Level1(next_pkid) //TODO: why only Level1
            }
        };

        let message = message.transform(Some(qos.clone()));
        let topic = message.topic.clone();
        let ref payload = *message.payload;

        let publish_packet = try!(self._generate_publish_packet(message.topic.clone(),
                                                                qos.clone(),
                                                                payload.clone()));

        match message.qos {
            QoSWithPacketIdentifier::Level0 => (),
            QoSWithPacketIdentifier::Level1(pkid) => self.outgoing_ack.push_back(message.clone()),
            QoSWithPacketIdentifier::Level2(pkid) => (),
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

// pub struct Subscriber {
//     subscribe_send: Sender<Vec<(TopicFilter, QualityOfService)>>,
//     message_recv: Receiver<Message>,
// }

// impl Subscriber {
//     pub fn subscribe(&self, topics: Vec<(TopicFilter, QualityOfService)>) {
//         debug!("---> Subscribing");
//         self.subscribe_send.send(topics);
//     }

//     pub fn receive(&self) -> Result<Message> {
//         debug!("Receive message wait <---");
//         let message = try!(self.message_recv.recv());
//         Ok(message)
//     }
// }

// impl Proxy {
//     pub fn await(self) -> Result<()> {
//         let mut proxy_client = ProxyClient {
//             addr: self.addr,
//             state: MqttClientState::Disconnected,
//             opts: self.opts.clone(),
//             stream: None,
//             session_present: self.session_present,
//             last_flush: Instant::now(),
//             last_pid: PacketIdentifier(0),
//             await_ping: false,
//             // Queues
//             incomming_pub: VecDeque::new(),
//             incomming_rec: VecDeque::new(),
//             incomming_rel: VecDeque::new(),
//             outgoing_ack: Arc::new(VecDeque::new()),
//             outgoing_rec: VecDeque::new(),
//             outgoing_comp: VecDeque::new(),
//         };

//         let subscribe_recv = self.subscribe_recv;
//         let message_send = self.message_send;
//         let pub_recv_dummy = self.pub_recv;
//         let (pub_send, pub_recv) = mioco::sync::mpsc::channel::<Message>();

//         mioco::start(move || {
//             let addr = proxy_client.addr;
//             let mut stream = proxy_client._reconnect(addr).unwrap();

//             // Mqtt connect packet send + connack packet await
//             match proxy_client._handshake() {
//                 Ok(_) => (),
//                 Err(e) => return Err(e),
//             };

//             // Has to reroute this way for using
//             // synchronous channels functionality
//             // TODO: remove this once there is synchronous channels
//             // in mioco
//             let outgoing_ack = proxy_client.outgoing_ack.clone();
//             mioco::spawn(move || {
//                 let mut outgoing_ack = outgoing_ack.lock().unwrap();
//                 loop {
//                     if outgoing_ack.len() < 10 {
//                         if let Ok(message) = pub_recv_dummy.recv() {
//                             //info!("message = {:?}", message);
//                             pub_send.send(message);
//                         }
//                     }else{
//                         thread::sleep(Duration::new(3, 0));
//                     }
//                 }

// });

//             let mut pingreq_timer = Timer::new();
//             // let mut retry_timer = Timer::new();

//             // let stream = match proxy_client.stream {
//             //     Some(ref mut s) => s,
//             //     None => return Err(Error::NoStreamError),
//             // };

//             loop {
//                 pingreq_timer.set_timeout(proxy_client.opts.keep_alive.unwrap() as i64 * 1000);
//                 // retry_timer.set_timeout(10 * 1000);

//                 select!(
//                         r:pingreq_timer => {
//                             match proxy_client.state {
//                                 MqttClientState::Connected | MqttClientState::Handshake => {
//                                     info!("@PING REQ");
//                                     if !proxy_client.await_ping {
//                                         let _ = proxy_client.ping();
//                                     } else {
//                                         panic!("awaiting for previous ping resp");
//                                     }
//                                 }
//                                 MqttClientState::Disconnected => {
//                                      for _ in 0..3 {
//                                          if let Err(e) = proxy_client._try_reconnect() {
//                                              continue;
//                                          }
//                                          break;
//                                      }
//                                 }
//                             }
//                         },

//                         r:stream => {
//                             let packet = match VariablePacket::decode(&mut stream) {
//                                 Ok(pk) => pk,
//                                 Err(err) => {
//                                     // maybe size=0 while reading indicating socket
//                                     // close at broker end
//                                     error!("Error in receiving packet {:?}", err);
//                                     proxy_client.state = MqttClientState::Disconnected;
//                                     loop {
//                                         match proxy_client._try_reconnect() {
//                                             Ok(_) => break,
//                                             Err(e) => {
//                                                 // return incase of disconnections
//                                                 // if retry method is not set
//                                                 match e {
//                                                     Error::NoReconnectTry => return Err(e),
//                                                     _ => (),
//                                                 }
//                                             }
//                                         }
//                                     }
//                                     continue;
//                                 }
//                             };

//                             trace!("   %%%RECEIVED PACKET {:?}", packet);
//                             match proxy_client.handle_packet(&packet){
//                                 Ok(message) => {
//                                     if let Some(m) = message {
//                                         message_send.send(*m);
//                                     }
//                                 },
//                                 Err(err) => panic!("error in handling packet. {:?}", err),
//                             };
//                         },

//                         r:subscribe_recv => {
//                             info!("@SUBSCRIBE REQUEST");
//                             if let Ok(topics) = subscribe_recv.try_recv(){
//                                 info!("request = {:?}", topics);
//                                 proxy_client._subscribe(topics);
//                             }
//                         },
