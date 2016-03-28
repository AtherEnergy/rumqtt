
use super::client::MqttClient;
use {TopicFilter, QualityOfService, Encodable};
use std::io::Write;
use packet::SubscribePacket;

impl MqttClient {
    pub fn subscribe(&mut self,
                     topics: Vec<(TopicFilter, QualityOfService)>)
                     -> Result<&Self, i32> {


        let mut connection = self.connection.lock().unwrap();
        let ref stream = connection.stream;

        let mut stream = match *stream {
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
}
