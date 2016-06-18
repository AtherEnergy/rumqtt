use std::time::{Duration, Instant};
use rand::{self, Rng};
use std::net::{SocketAddr, ToSocketAddrs};
use error::{Error, Result};
use std::collections::VecDeque;
use std::io::{Read, Write};
use mioco::tcp::TcpStream;
use mqtt::{Encodable, Decodable, QualityOfService};
use mqtt::packet::*;
use mqtt::control::variable_header::ConnectReturnCode;
use mioco::timer::Timer;
use mioco;

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

    pub fn connect<A: ToSocketAddrs>(mut self, addr: A) -> Result<Client> {
        if self.client_id == None {
            self.generate_client_id();
        }

        let addr = try!(addr.to_socket_addrs()).next().expect("Socket address is broken");

        // info!(" Connecting to {}", addr);
        // let stream = try!(self._reconnect(addr));

        let mut client = Client {
            addr: addr,
            state: MqttClientState::Disconnected,
            opts: self,
            stream: None,
            session_present: false,
            last_flush: Instant::now(),
            await_ping: false,
        };

        Ok(client)
    }


    fn _generate_connect_packet(&self) -> Result<Vec<u8>> {
        let mut connect_packet = ConnectPacket::new("MQTT".to_owned(),
                                                    self.client_id.clone().unwrap());
        connect_packet.set_clean_session(self.clean_session);
        connect_packet.set_keep_alive(self.keep_alive.unwrap());

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

pub struct Client {
    addr: SocketAddr,
    state: MqttClientState,
    opts: ClientOptions,
    stream: Option<TcpStream>,
    session_present: bool,
    last_flush: Instant,
    await_ping: bool,
}

impl Client {
    pub fn await(&mut self) {
        // let keep_alive = self.opts.keep_alive.unwrap() as i64;
        // let addr = self.addr;
        let mut client = Client {
            addr: self.addr,
            state: MqttClientState::Disconnected,
            opts: self.opts.clone(),
            stream: None,
            session_present: self.session_present,
            last_flush: Instant::now(),
            await_ping: false,
        };

        mioco::start(move || {
            let mut stream = Self::_reconnect(client.addr).unwrap();
            client.stream = Some(stream.try_clone().unwrap());

            // Mqtt connect packet send + connack packet await
            match client._handshake() {
                Ok(_) => (),
                Err(e) => return Err(e),
            };

            loop {
                let mut timer = Timer::new();
                timer.set_timeout(client.opts.keep_alive.unwrap() as i64 * 1000);

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
                            match &packet {
                                &VariablePacket::SubackPacket(ref ack) => {
                                    if ack.packet_identifier() != 10 {
                                        error!("SUBACK packet identifier not match");
                                    }
                                    else {
                                        println!("Subscribed!");
                                    }
                                },

                                &VariablePacket::PingrespPacket(ref ack) => {
                                    client.await_ping = false;
                                },

                                _ => {}
                            }
                        },
                        r:timer => {
                            info!("PING REQ");
                            if !client.await_ping {
                                let _ = client.ping();
                            } else {
                                panic!("awaiting for previous ping resp");
                            }
                        },

                );
            } //loop end
            Ok(())
        }); //mioco end

    }


    fn _reconnect(addr: SocketAddr) -> Result<TcpStream> {
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
            panic!("Failed to connect to server, return code {:?}", connack.connect_return_code());
        } else {
            self.state = MqttClientState::Connected;
        }

        Ok(())
    }

    fn _connect(&mut self) -> Result<()> {
        let connect = try!(self.opts._generate_connect_packet());
        try!(self._write_packet(connect));
        self._flush()
    }

    pub fn ping(&mut self) -> Result<()> {
        debug!("       Pingreq");
        let ping = try!(self.opts._generate_pingreq_packet());
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
}
