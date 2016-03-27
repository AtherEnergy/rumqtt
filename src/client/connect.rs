extern crate time;

use super::client::MqttClient;
use packet::*;
use std::net::TcpStream;
use std::thread;
use std::io::Write;
use {Encodable, Decodable};
use std::sync::mpsc::{self, Sender, Receiver};
use std::sync::{Arc, Mutex};
use control::variable_header::ConnectReturnCode;
use std::str;
use std::sync::atomic::Ordering;

#[derive(Debug)]
pub enum MqttErrors {
    EncodeError,
    WriteError,
    Error, // std io errors
    ConnAckError,
}


pub type SendableFn = Arc<Mutex<(Fn(&str, &str) + Send + Sync + 'static)>>;


impl MqttClient {
    pub fn connect(&mut self, host: &str) -> Result<&Self, MqttErrors> {
        // Create a TCP stream
        let mut stream = match TcpStream::connect(host) {
            Ok(result) => result,
            Err(_) => {
                return Err(MqttErrors::Error);
            }
        };

        // Creating a mqtt connection packet
        let mut conn = ConnectPacket::new("MQTT".to_owned(), self.options.id.clone());
        conn.set_clean_session(self.options.clean_session);
        conn.set_keep_alive(self.options.keep_alive);
        let mut buf = Vec::new();

        match conn.encode(&mut buf) {
            Ok(result) => result,
            Err(_) => {
                return Err(MqttErrors::EncodeError);
            }
        };

        match stream.write_all(&buf[..]) { 
            Ok(result) => result,
            Err(_) => {
                return Err(MqttErrors::Error);
            }
        };

        let connack = ConnackPacket::decode(&mut stream).unwrap();
        trace!("CONNACK {:?}", connack);

        if connack.connect_return_code() != ConnectReturnCode::ConnectionAccepted {

            return Err(MqttErrors::ConnAckError);

        } else {
            // If mqtt connection is successful, start a thread to send
            // ping responses and handle incoming messages
            let mut stream_clone = match stream.try_clone() {
                Ok(s) => s,
                Err(_) => return Err(MqttErrors::Error),
            };
            let mut stream_clone2 = match stream.try_clone() {
                Ok(s) => s,
                Err(_) => return Err(MqttErrors::Error),
            };
            self.stream = Some(stream);

            let (tx, rx): (Sender<SendableFn>, Receiver<SendableFn>) = mpsc::channel();
            self.msg_callback = Some(tx);

            let publish_queue = self.publish_queue.queue.clone();
            let keep_alive = self.options.keep_alive;
            // let last_ping_time = self.last_ping_time;
            thread::spawn(move || {
                let mut current_message_callback: Option<SendableFn> = None;
                let mut last_message_callback: Option<SendableFn> = None;
                loop {
                    let message_callback = rx.try_recv();

                    current_message_callback = message_callback.ok().map(|cb| {
                        last_message_callback = Some(cb.clone());
                        cb
                    });

                    // blocking
                    let packet = match VariablePacket::decode(&mut stream_clone) {
                        Ok(pk) => pk,
                        Err(err) => {
                            error!("Error in receiving packet {:?}", err);
                            continue;
                        }
                    };
                    // println!("#### {:?} ####", packet);

                    match &packet {

                        /// Receive ping reqests and send ping responses
                        &VariablePacket::PingrespPacket(..) => {
                            println!("Received PINGRESP from broker");
                            // let pingresp = PingrespPacket::new();
                            // pingresp.encode(&mut stream_clone).unwrap();

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
                        /// Receives suback packet and verifies it with sub packet id
                        &VariablePacket::PubackPacket(ref ack) => {
                            let pkid = ack.packet_identifier();
                            let mut publish_queue = publish_queue.lock().unwrap();
                            let mut split_index: Option<usize> = None;
                            for (i, v) in publish_queue.iter().enumerate() {
                                if v.0 == pkid {
                                    split_index = Some(i);
                                }
                            }

                            if split_index.is_some() {
                                let split_index = split_index.unwrap();
                                let mut list2 = publish_queue.split_off(split_index);
                                list2.pop_front();
                                publish_queue.append(&mut list2);
                            }
                            println!("pub ack for {}. queue --> {:?}",
                                     ack.packet_identifier(),
                                     *publish_queue);
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



            // ping request thread. new thread since the above thread is blocking
            let last_ping_time = self.last_ping_time.clone();
            thread::spawn(move || {

                loop {
                    let lpt: i64;

                    {
                        let last_ping_time = last_ping_time.lock().unwrap();
                        lpt = *last_ping_time;
                    }
                    // pingreq
                    let current_timestamp = time::get_time().sec;
                    println!("current_timestamp = {:?}", current_timestamp);
                    println!("nextping_timestamp = {:?}",
                             lpt + (keep_alive as f32 * 0.8) as i64);
                    if keep_alive > 0 &&
                       current_timestamp >= lpt + (keep_alive as f32 * 0.8) as i64 {
                        println!("Sending PINGREQ");

                        let pingreq_packet = PingreqPacket::new();

                        let mut buf = Vec::new();
                        pingreq_packet.encode(&mut buf).unwrap();
                        stream_clone2.write_all(&buf[..]);

                        {
                            let mut last_ping_time = last_ping_time.lock().unwrap();
                            *last_ping_time = current_timestamp;
                        }
                    }

                }
            });
        }

        Ok(self)
    }
}
