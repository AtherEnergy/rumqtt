use super::mclient::MqttClient;
use packet::*;
use std::net::TcpStream;
use std::thread;
use std::io::Write;
use {Encodable, Decodable};
use std::sync::mpsc::{self, Sender, Receiver};
use std::sync::{Arc, Mutex};
use control::variable_header::ConnectReturnCode;
use std::str;


pub type SendableFn = Arc<Mutex<(Fn(&str, &str) + Send + Sync + 'static)>>;


impl MqttClient {
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
}
