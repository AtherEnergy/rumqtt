use std::time::{Duration, Instant};
use rand::{self, Rng};
use std::net::{SocketAddr, ToSocketAddrs};
use error::{Error, Result};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::str;
use mioco::tcp::TcpStream;
use mqtt::{Encodable, Decodable, QualityOfService, TopicFilter};
use mqtt::packet::*;
use mqtt::control::variable_header::ConnectReturnCode;
use mioco::timer::Timer;
use mioco;
use mioco::sync::mpsc::{Sender, Receiver};

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

    pub fn connect<A: ToSocketAddrs>(mut self, addr: A) -> Result<(Proxy, Client)> {
        if self.client_id == None {
            self.generate_client_id();
        }

        let addr = try!(addr.to_socket_addrs()).next().expect("Socket address is broken");
        let (sub_send, sub_recv) = mioco::sync::mpsc::channel::<Vec<(TopicFilter,
                                                                     QualityOfService)>>();
        // info!(" Connecting to {}", addr);
        // let stream = try!(self._reconnect(addr));

        let proxy = Proxy {
            addr: addr,
            opts: self,
            stream: None,
            session_present: false,
            subscribe_recv: sub_recv,
        };

        let client = Client { subscribe_send: sub_send };

        Ok((proxy, client))
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

pub struct Proxy {
    addr: SocketAddr,
    opts: ClientOptions,
    stream: Option<TcpStream>,
    session_present: bool,
    subscribe_recv: Receiver<Vec<(TopicFilter, QualityOfService)>>,
}

pub struct ProxyClient {
    addr: SocketAddr,
    state: MqttClientState,
    opts: ClientOptions,
    stream: Option<TcpStream>,
    session_present: bool,
    last_flush: Instant,
    await_ping: bool,
}

pub struct Client {
    subscribe_send: Sender<Vec<(TopicFilter, QualityOfService)>>,
}

impl Client {
    pub fn subscribe(&self, topics: Vec<(TopicFilter, QualityOfService)>) {
        info!("Subscribing");
        self.subscribe_send.send(topics);
    }
}

impl Proxy {
    pub fn await(self) {
        let mut proxy_client = ProxyClient {
            addr: self.addr,
            state: MqttClientState::Disconnected,
            opts: self.opts.clone(),
            stream: None,
            session_present: self.session_present,
            last_flush: Instant::now(),
            await_ping: false,
        };
        let subscribe_recv = self.subscribe_recv;

        mioco::start(move || {
            let addr = proxy_client.addr;
            let mut stream = proxy_client._reconnect(addr).unwrap();
            proxy_client.stream = Some(stream.try_clone().unwrap());

            // Mqtt connect packet send + connack packet await
            match proxy_client._handshake() {
                Ok(_) => (),
                Err(e) => return Err(e),
            };

            let mut timer = Timer::new();

            loop {
                timer.set_timeout(proxy_client.opts.keep_alive.unwrap() as i64 * 1000);
                // let subscribe_recv = subscribe_recv;
                select!(
                        r:stream => {
                            let packet = match VariablePacket::decode(&mut stream) {
                                Ok(pk) => pk,
                                Err(err) => {
                // maybe size=0 while reading indicating socket close at broker end
                                    error!("Error in receiving packet {:?}", err);
                                    continue;
                                }
                            };
                            trace!("PACKET {:?}", packet);
                            proxy_client.handle_packet(&packet);

                        },
                        r:timer => {
                            info!("PING REQ");
                            if !proxy_client.await_ping {
                                let _ = proxy_client.ping();
                            } else {
                                panic!("awaiting for previous ping resp");
                            }
                        },
                // r:subscribe_recv => {
                //     info!("subscribe request");
                // },
                );
            } //loop end
            Ok(())
        }); //mioco end

    }
}


impl ProxyClient {
    fn handle_packet(&mut self, packet: &VariablePacket) {
        match packet {
            &VariablePacket::ConnackPacket(ref pubrec) => {}

            &VariablePacket::SubackPacket(ref ack) => {
                if ack.packet_identifier() != 10 {
                    error!("SUBACK packet identifier not match");
                } else {
                    println!("Subscribed!");
                }
            }

            &VariablePacket::PingrespPacket(..) => {
                self.await_ping = false;
            }

            /// Receives disconnect packet
            &VariablePacket::DisconnectPacket(..) => {
                // TODO
            }

            /// Receives puback packet and verifies it with sub packet id
            &VariablePacket::PubackPacket(ref ack) => {
                let pkid = ack.packet_identifier();

                // let mut connection = self.connection.lock().unwrap();
                // let ref mut publish_queue = connection.queue;

                // let mut split_index: Option<usize> = None;
                // for (i, v) in publish_queue.iter().enumerate() {
                //     if v.pkid == pkid {
                //         split_index = Some(i);
                //     }
                // }

                // if split_index.is_some() {
                //     let split_index = split_index.unwrap();
                //     let mut list2 = publish_queue.split_off(split_index);
                //     list2.pop_front();
                //     publish_queue.append(&mut list2);
                // }
                // println!("pub ack for {}. queue --> {:?}",
                //         ack.packet_identifier(),
                //         publish_queue);
            }

            /// Receives publish packet
            &VariablePacket::PublishPacket(ref publ) => {
                let msg = match str::from_utf8(&publ.payload()[..]) {
                    Ok(msg) => msg,
                    Err(err) => {
                        error!("Failed to decode publish message {:?}", err);
                        return;
                    }
                };
            }

            &VariablePacket::PubrecPacket(ref pubrec) => {}

            &VariablePacket::PubrelPacket(ref pubrel) => {}

            &VariablePacket::PubcompPacket(ref pubcomp) => {}

            &VariablePacket::UnsubackPacket(ref pubrec) => {}

            _ => {}
        }
    }


    fn _reconnect(&mut self, addr: SocketAddr) -> Result<TcpStream> {
        // Raw tcp connect
        let stream = try!(TcpStream::connect(&addr));
        Ok(stream)
    }


    fn _handshake(&mut self) -> Result<()> {
        self.state = MqttClientState::Handshake;
        // send CONNECT
        try!(self._connect());

        // wait CONNACK
        let stream = match self.stream {
            Some(ref mut s) => s,
            None => return Err(Error::NoStreamError),
        };
        let connack = ConnackPacket::decode(stream).unwrap();
        trace!("CONNACK {:?}", connack);

        if connack.connect_return_code() != ConnectReturnCode::ConnectionAccepted {
            panic!("Failed to connect to server, return code {:?}",
                   connack.connect_return_code());
        } else {
            self.state = MqttClientState::Connected;
        }

        Ok(())
    }

    fn _connect(&mut self) -> Result<()> {
        let connect = try!(self._generate_connect_packet());
        try!(self._write_packet(connect));
        self._flush()
    }

    pub fn ping(&mut self) -> Result<()> {
        debug!("       Pingreq");
        let ping = try!(self._generate_pingreq_packet());
        self.await_ping = true;
        try!(self._write_packet(ping));
        self._flush()
    }

    fn _flush(&mut self) -> Result<()> {
        // TODO: in case of disconnection, trying to reconnect
        let stream = match self.stream {
            Some(ref mut s) => s,
            None => return Err(Error::NoStreamError),
        };

        try!(stream.flush());
        self.last_flush = Instant::now();
        Ok(())
    }

    #[inline]
    fn _write_packet(&mut self, packet: Vec<u8>) -> Result<()> {
        trace!("{:?}", packet);
        let stream = match self.stream {
            Some(ref mut s) => s,
            None => return Err(Error::NoStreamError),
        };

        stream.write_all(&packet).unwrap();
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

        pingreq_packet.encode(&mut buf).unwrap();
        match pingreq_packet.encode(&mut buf) {
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
}