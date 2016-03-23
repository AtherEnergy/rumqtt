use std::default::Default;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{self, Sender, Receiver};
use std::net::TcpStream;
use std::io::{self, Write};
use std::thread;
use std::str;

use packet::*;
use {Encodable, Decodable, QualityOfService};
use {TopicFilter, TopicName};
use control::variable_header::ConnectReturnCode;

pub type SendableFn = Arc<Mutex<(Fn(&str, &str) + Send + Sync + 'static)>>;

pub struct MqttClient {
    pub id: String,
    pub keep_alive: i32,
    pub clean_session: bool,
    pub stream: Option<TcpStream>,
    pub tx: Option<Sender<SendableFn>>,
}

impl Default for MqttClient {
    fn default() -> MqttClient {
        MqttClient {
            id: "".to_string(),
            keep_alive: 3,
            clean_session: true,
            stream: None,
            tx: None,
        }
    }
}

impl MqttClient {
    pub fn new(id: &str) -> MqttClient {
        let client = MqttClient { id: id.to_string(), ..Default::default() };
        client
    }

    // TODO: Implement keep_alive in lower layers
    pub fn keep_alive(mut self, val: i32) -> Self {
        self.keep_alive = val;
        self
    }

    pub fn clean_session(mut self, val: bool) -> Self {
        self.clean_session = val;
        self
    }

    pub fn connect(&mut self, host: &str) -> Result<&Self, i32> {
        let mut stream = match TcpStream::connect(host) {
            Ok(result) => result,
            Err(_) => {
                return Err(-1);
            }
        };

        // Creating a mqtt connection packet
        let mut conn = ConnectPacket::new("MQTT".to_owned(), self.id.clone());
        conn.set_clean_session(self.clean_session);
        let mut buf = Vec::new();

        match conn.encode(&mut buf) {
            Ok(result) => result,
            Err(_) => {
                return Err(-2);
            }
        };

        match stream.write_all(&buf[..]) { 
            Ok(result) => result,
            Err(_) => {
                return Err(-3);
            }
        };

        let connack = ConnackPacket::decode(&mut stream).unwrap();
        trace!("CONNACK {:?}", connack);

        if connack.connect_return_code() != ConnectReturnCode::ConnectionAccepted {

            return Err(-5);

        } else {
            // If mqtt connection is successful, start a thread to send
            // ping responses and handle incoming messages
            let mut stream_clone = match stream.try_clone() {
                Ok(s) => s,
                Err(_) => return Err(-6),
            };
            self.stream = Some(stream);

            let (tx, rx): (Sender<SendableFn>, Receiver<SendableFn>) = mpsc::channel();
            self.tx = Some(tx);

            thread::spawn(move || {
                let mut current_message_callback: Option<SendableFn> = None;
                let mut last_message_callback: Option<SendableFn> = None;

                loop {

                    let message_callback = rx.try_recv();

                    current_message_callback = message_callback.ok().map(|cb| {
                        last_message_callback = Some(cb.clone());
                        cb
                    });


                    let packet = match VariablePacket::decode(&mut stream_clone) {
                        Ok(pk) => pk,
                        Err(err) => {
                            error!("Error in receiving packet {:?}", err);
                            continue;
                        }
                    };

                    println!("#### {:?} ####", packet);

                    match &packet {

                        /// Receive ping reqests and send ping responses
                        &VariablePacket::PingreqPacket(..) => {
                            println!("keep alive");
                            let pingresp = PingrespPacket::new();
                            pingresp.encode(&mut stream_clone).unwrap();

                            // TODO: Is encode sending ping responses to the broker ??
                        }
                        /// Receives disconnect packet
                        &VariablePacket::DisconnectPacket(..) => {
                            println!("### Received disconnect");
                            break;
                            // TODO: Do we need to notify main thread about this ?
                        }
                        /// Receives suback packet and verifies it with sub packet id
                        &VariablePacket::SubackPacket(ref ack) => {
                            if ack.packet_identifier() != 11 {
                                panic!("SUBACK packet identifier not match");
                            }

                            println!("Subscribed!!!!!");
                        }
                        /// Receives publish packet
                        &VariablePacket::PublishPacket(ref publ) => {
                            let msg = match str::from_utf8(&publ.payload()[..]) {
                                Ok(msg) => msg,
                                Err(err) => {
                                    error!("Failed to decode publish message {:?}", err);
                                    continue;
                                }
                            };
                            // println!("PUBLISH ({}): {}", publ.topic_name(), msg);



                            match current_message_callback {
                                Some(ref cb) => {
                                    let callback = cb.lock().unwrap();
                                    (*callback)(publ.topic_name(), msg)
                                }
                                None => {
                                    match last_message_callback {
                                        Some(ref cb) => {
                                            let callback = cb.lock().unwrap();
                                            (*callback)(publ.topic_name(), msg)
                                        }
                                        None => (),
                                    }
                                }
                            }
                        }
                        _ => {
                            // Ignore other packets in pub client
                        }
                    }
                }
            });
        }

        Ok(self)
    }

    pub fn publish(&self, topic: &str, message: &str) -> Result<&Self, i32> {

        let mut stream = match self.stream {
            Some(ref s) => s,
            None => return Err(-9),
        };

        let topic = topic.to_string();
        let topic = match TopicName::new(topic) {
            Ok(n) => n,
            Err(_) => return Err(-8),
        };

        let publish_packet = PublishPacket::new(topic,
                                                QoSWithPacketIdentifier::Level0,
                                                message.as_bytes().to_vec());
        let mut buf = Vec::new();
        publish_packet.encode(&mut buf).unwrap();

        match stream.write_all(&buf[..]) {
            Ok(result) => result,
            Err(_) => {
                return Err(-9);
            }
        }
        Ok(self)
    }

    pub fn subscribe(&mut self,
                     topics: Vec<(TopicFilter, QualityOfService)>)
                     -> Result<&Self, i32> {

        let mut stream = match self.stream {
            Some(ref s) => s,
            None => return Err(-10),
        };

        let sub = SubscribePacket::new(11, topics);
        let mut buf = Vec::new();
        sub.encode(&mut buf).unwrap();

        match stream.write_all(&buf[..]) {
            Ok(result) => result,
            Err(_) => {
                return Err(-9);
            }
        }

        Ok(self)
    }

    pub fn on_message<F>(&self, callback: F) -> Result<&Self, i32>
        where F: Fn(&str, &str) + Send + Sync + 'static
    {
        let callback = Arc::new(Mutex::new(callback));

        match self.tx {
            Some(ref t) => t.send(callback),
            None => return Err(-10),
        };

        Ok(self)
    }
}
