extern crate time;

use packet::{PublishPacket, QoSWithPacketIdentifier};
use super::client::{MqttClient, MqttConnection, PublishMessage};
use {TopicName, Encodable, QualityOfService};
use std::io::Write;
use std::sync::atomic::Ordering;

pub enum PublishError {
    TopicNameError,
    Error, // std io error
    StreamError,
}

impl MqttClient{
    pub fn publish(&mut self,
                   topic: &str,
                   message: &str,
                   qos: QualityOfService)
                   -> Result<&Self, PublishError> {

        let mut connection_guard = self.connection.lock().unwrap();

        let MqttConnection{ref mut stream,
             ref mut current_pkid,
             ref mut queue,
             ref mut length,
             ref mut retry_time} = *connection_guard;

        let mut stream = match *stream {
            Some(ref s) => s,
            None => return Err(PublishError::StreamError),
        };

        let topic = topic.to_string();
        let topic = match TopicName::new(topic) {
            Ok(n) => n,
            Err(_) => return Err(PublishError::TopicNameError),
        };

        let mut pkid: u16 = 1;
        let qos_final = match qos {
            QualityOfService::Level0 => QoSWithPacketIdentifier::Level0,
            QualityOfService::Level1 | QualityOfService::Level2 => {
                pkid = current_pkid.fetch_add(1, Ordering::SeqCst) as u16;
                QoSWithPacketIdentifier::Level1(pkid)
            }
        };

        let publish_packet = PublishPacket::new(topic, qos_final, message.as_bytes().to_vec());

        let mut buf = Vec::new();
        publish_packet.encode(&mut buf).unwrap();

        match stream.write_all(&buf[..]) {
            Ok(result) => {

                match qos {
                    QualityOfService::Level1 => {
                        let timestamp = time::get_time().sec;
                        queue.push_back(PublishMessage {
                            pkid: pkid,
                            timestamp: timestamp,
                            message: message.to_string(),
                        });
                        println!("publish done. queue --> {:?}", queue);
                    }
                    _ => (),
                }
                result
            }
            Err(_) => {
                return Err(PublishError::Error);
            }
        }
        Ok(self)
    }
}
