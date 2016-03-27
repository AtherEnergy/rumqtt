extern crate time;

use packet::{PublishPacket, QoSWithPacketIdentifier};
use super::client::MqttClient;
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

        let mut stream = match self.stream {
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
            QualityOfService::Level1 => {
                pkid = self.publish_queue.current_pkid.fetch_add(1, Ordering::SeqCst) as u16;
                QoSWithPacketIdentifier::Level1(pkid)
            }
            QualityOfService::Level2 => {
                pkid = self.publish_queue.current_pkid.fetch_add(1, Ordering::SeqCst) as u16;
                QoSWithPacketIdentifier::Level2(pkid)
            }
        };

        let publish_packet = PublishPacket::new(topic, qos_final, message.as_bytes().to_vec());

        let mut buf = Vec::new();
        publish_packet.encode(&mut buf).unwrap();

        match stream.write_all(&buf[..]) {
            Ok(result) => {
                // self.last_ping_time.store(time::get_time().sec as usize, Ordering::Relaxed);
                {
                    let mut last_ping_time = self.last_ping_time.lock().unwrap();
                    *last_ping_time = time::get_time().sec;
                }
                match qos {
                    QualityOfService::Level1 => {
                        let mut publish_queue = self.publish_queue.queue.lock().unwrap();
                        let timestamp = time::get_time().sec;
                        publish_queue.push_back((pkid, timestamp));
                        println!("publish done. queue --> {:?}", *publish_queue);
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
