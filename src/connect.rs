use mqtt::{Encodable, Decodable, QualityOfService};
use mqtt::packet::*;
use mqtt::control::variable_header::ConnectReturnCode;
use super::client::{MqttClient, MqttConnection};
use std::net::TcpStream;
use std::thread;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::str;
use std::time::Duration;
use time;

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

        // Connect using underlying connection object. Scoped since this
        // connection object will be used in other threads
        {
            let ref mut mqtt_connection = *self.connection.lock().unwrap();
            match mqtt_connection.connect(host) {
                Ok(_) => (),
                Err(_) => return Err(MqttErrors::Error),
            }
        }

        let _client_pkt_hndlr_thrd = self.clone();

        // incoming packet handler and reconnection thread
        thread::spawn(move || {

            // get updated underlying stream after reconnection
            'reconnection: loop {
                // get underlying tcpstream clone from connection object
                let mut _stream_pkt_hndlr_thrd = {
                    let ref connection = *_client_pkt_hndlr_thrd.connection.lock().unwrap();
                    let stream = match connection.stream {
                        Some(ref s) => s,
                        None => panic!("No stream found in the connectino"),
                    };
                    stream.try_clone().unwrap()
                };

                loop {
                    // blocking
                    let packet = match VariablePacket::decode(&mut _stream_pkt_hndlr_thrd) {
                        Ok(pk) => pk,
                        Err(err) => {
                            println!("error in receiving packet {:?}", err);
                            let mut connection = _client_pkt_hndlr_thrd.connection.lock().unwrap();
                            if connection.retry(3) {
                                continue 'reconnection;
                            } else {
                                // do an on weird callback here so that user can handle disconnection
                                unimplemented!();
                                continue; //remove this later
                            }

                        }
                    };
                    _client_pkt_hndlr_thrd.handle_packet(&packet);
                } // packet handle
            } // reconnection
        });


        // This thread goes through entire queue to republish timedout publishes
        let t2_mqtt_client = self.clone();
        let mut t3_mqtt_client = self.clone(); //super ugly
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::new(5, 0)); //TODO change this sleep to retry time.
                let mut g_mqtt_connection = t2_mqtt_client.connection.lock().unwrap();
                let MqttConnection { ref mut queue, ref mut retry_time, .. } = *g_mqtt_connection;

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


        let _client_ping_thrd = self.clone();
        let keep_alive = {
            let connection = self.connection.lock().unwrap();
            connection.options.keep_alive
        };

        // ping request thread. new thread since the above thread is blocking
        // TODO: Check ping responses here. Do something if there is no response for a request
        if keep_alive > 0 {
            thread::spawn(move || {
                let sleep_time = (keep_alive as f32 * 0.8) as u64;
                loop {
                    let pingreq_packet = PingreqPacket::new();
                    let mut buf = Vec::new();

                    pingreq_packet.encode(&mut buf).unwrap();

                    {
                        let ref connection = *_client_ping_thrd.connection.lock().unwrap();
                        let mut stream = match connection.stream {
                            Some(ref s) => s,
                            None => panic!("No stream found in the connectino"),
                        };
                        stream.write_all(&buf[..]);
                    }
                    thread::sleep(Duration::new(sleep_time, 0));
                }
            });
        }

        Ok(self)
    }


    // TODO: Add appropriate return
    fn handle_packet(&self, packet: &VariablePacket) {
        // println!("#### {:?} ####", packet);

        match packet {

            /// Receives disconnect packet
            &VariablePacket::DisconnectPacket(..) => {
                // TODO: Do we need to notify main thread about this ?
            }

            /// Receives puback packet and verifies it with sub packet id
            &VariablePacket::PubackPacket(ref ack) => {
                let pkid = ack.packet_identifier();

                let mut connection = self.connection.lock().unwrap();
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
                    Err(_) => {
                        // error!("Failed to decode publish message {:?}", err);
                        return;
                    }
                };

                let ref message_callback = self.msg_callback;
                match message_callback.as_ref() {
                    Some(ref cb) => {
                        let callback = cb.lock().unwrap();
                        (*callback)(publ.topic_name(), msg)
                    }
                    None => (),
                };
            }

            _ => {
                // Ignore other packets in pub client
            }

        }
    }
}

impl MqttConnection {
    fn connect(&mut self, host: &str) -> Result<&Self, MqttErrors> {

        self.host = host.to_string();
        let ref connection_options = self.options;

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
        // trace!("CONNACK {:?}", connack);

        if connack.connect_return_code() != ConnectReturnCode::ConnectionAccepted {
            Err(MqttErrors::ConnAckError)
        } else {
            self.stream = Some(stream);
            Ok(self)
        }
    }

    fn retry(&mut self, count: i32) -> bool {

        for _ in 0..count {
            println!("trying to reconnect ..");
            let host = self.host.clone();
            match self.connect(&host) {
                Ok(_) => {
                    println!("reconnection successful ...");
                    break;
                }
                Err(_) => {
                    thread::sleep(Duration::new(3, 0));
                    continue;
                }
            }
        }

        true
    }
}
