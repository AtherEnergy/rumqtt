extern crate time;

use super::client::{MqttClient, MqttConnection, MqttConnectionOptions};
use packet::*;
use std::net::TcpStream;
use std::thread;
use std::io::Write;
use {Encodable, Decodable, QualityOfService};
use std::sync::mpsc::{self, Sender, Receiver};
use std::sync::{Arc, Mutex};
use control::variable_header::ConnectReturnCode;
use std::str;
use std::sync::atomic::Ordering;
use std::time::Duration;

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

        let mut stream_clone: TcpStream;
        let mut stream_clone2: TcpStream;
        {
            let ref mut mqtt_connection = *self.connection.lock().unwrap();
            match mqtt_connection.connect(host, self.options.clone()) {
                Ok(_) => {
                    stream_clone = match mqtt_connection.stream {
                        Some(ref s) => s.try_clone().unwrap(),
                        None => return Err(MqttErrors::Error),
                    };

                    stream_clone2 = match mqtt_connection.stream {
                        Some(ref s) => s.try_clone().unwrap(),
                        None => return Err(MqttErrors::Error),
                    };

                }
                Err(_) => return Err(MqttErrors::Error),
            }
        }

        let (tx, rx): (Sender<SendableFn>, Receiver<SendableFn>) = mpsc::channel();
        self.msg_callback = Some(tx);

        // let publish_queue = self.publish_queue.queue.clone();
        let keep_alive = self.options.keep_alive;
        // let last_ping_time = self.last_ping_time;
        let t1_mqtt_client = self.clone();
        let mut t7_mqtt_client = self.clone();
        let h = host.to_string();
        let options = self.options.clone();

        thread::spawn(move || {
            let mut current_message_callback: Option<SendableFn> = None;
            let mut last_message_callback: Option<SendableFn> = None;
            let mut connect_retry_count = 0;
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
                        println!("error in receiving packet {:?}", err);
                        if connect_retry_count < 2 {
                            connect_retry_count += 1;
                            thread::sleep(Duration::new(1, 0));
                            continue;
                        }

                        // connection retry loop
                        loop {
                            println!("trying to reconnect ..");
                            let ref mut connection = *t7_mqtt_client.connection
                                                                    .lock()
                                                                    .unwrap();
                            match connection.connect(&h, options.clone()) {
                                Ok(result) => {
                                    connect_retry_count = 0;
                                    println!("reconnection successful ...");
                                    break;
                                }
                                Err(_) => {
                                    thread::sleep(Duration::new(3, 0));
                                    continue;
                                }
                            }
                        }


                        // reset current connections stream with new one
                        {
                            let ref mut connection = *t7_mqtt_client.connection
                                                                    .lock()
                                                                    .unwrap();
                            stream_clone = match connection.stream {
                                Some(ref mut s) => s.try_clone().unwrap(),
                                None => continue,
                            };
                        }

                        continue;

                    }
                };

                // // println!("#### {:?} ####", packet);

                match &packet {

                    /// Receives disconnect packet
                    &VariablePacket::DisconnectPacket(..) => {
                        println!("### Received disconnect");
                        break;
                        // TODO: Do we need to notify main thread about this ?
                    }

                    /// Receives puback packet and verifies it with sub packet id
                    &VariablePacket::PubackPacket(ref ack) => {
                        let pkid = ack.packet_identifier();

                        let mut connection = t1_mqtt_client.connection.lock().unwrap();
                        let ref mut publish_queue = connection.queue;

                        let mut split_index: Option<usize> = None;
                        for (i, v) in publish_queue.iter().enumerate() {
                            if v.pkid == pkid {
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
                                 publish_queue);
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

        // This thread goes through entire queue to republish timedout publishes
        let mut t2_mqtt_client = self.clone();
        let mut t3_mqtt_client = self.clone(); //super ugly
        thread::spawn(move || {
                loop {
                    thread::sleep(Duration::new(5, 0)); //TODO change this sleep to retry time.
                    let mut g_mqtt_connection = t2_mqtt_client.connection.lock().unwrap();
                    let MqttConnection{ref mut queue, ref mut retry_time, ..} = *g_mqtt_connection;

                    let current_timestamp = time::get_time().sec;
                    let mut split_index: Option<usize> = None;
                    for (i, v) in queue.iter().enumerate() {
                        if current_timestamp - v.timestamp > *retry_time as i64 {
                            split_index = Some(i);
                            println!("republishing pkid {:?} message ...", v.pkid);
                            t3_mqtt_client.publish(&v.topic, &v.message, QualityOfService::Level1); //move this down ?
                        }
                    }

                    if split_index.is_some() {
                        let split_index = split_index.unwrap();
                        let mut list2 = queue.split_off(split_index);
                        list2.pop_front();
                        queue.append(&mut list2);
                    }
                }
            });


        // ping request thread. new thread since the above thread is blocking
        // TODO: Check ping responses here. Do something if there is no response for a request
        thread::spawn(move || {
            let mut last_ping_time = 0;
            let mut next_ping_time = 0;
            loop {

                next_ping_time = last_ping_time + (keep_alive as f32 * 0.9) as i64;
                // pingreq
                let current_timestamp = time::get_time().sec;

                if keep_alive > 0 && current_timestamp >= next_ping_time {

                    let pingreq_packet = PingreqPacket::new();

                    let mut buf = Vec::new();
                    pingreq_packet.encode(&mut buf).unwrap();
                    stream_clone2.write_all(&buf[..]);

                    last_ping_time = current_timestamp;
                }
            }
        });


        Ok(self)
    }

}

impl MqttConnection {
    fn connect(&mut self,
               host: &str,
               connection_options: MqttConnectionOptions)
               -> Result<&Self, MqttErrors> {
        let mut stream = match TcpStream::connect(host) {
            Ok(result) => result,
            Err(_) => {
                return Err(MqttErrors::Error);
            }
        };

        // Creating a mqtt connection packet
        let mut conn = ConnectPacket::new("MQTT".to_owned(), connection_options.id.clone());
        conn.set_clean_session(connection_options.clean_session);
        conn.set_keep_alive(connection_options.keep_alive);
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
            Err(MqttErrors::ConnAckError)
        } else {
            self.stream = Some(stream);
            Ok(self)
        }
    }
}
