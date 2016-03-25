use packet::{PublishPacket, QoSWithPacketIdentifier};
use super::mclient::MqttClient;
use {TopicName, Encodable};
use std::io::Write;

impl MqttClient{
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
}
