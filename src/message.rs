
use std::sync::Arc;

use mqtt::topic_name::TopicName;
use mqtt::packet::*;
use error::Result;



#[derive(Debug, Clone)] //TODO: add Clone here
pub struct Message {
    pub topic: TopicName,
    pub retain: bool,
    // Only for QoS 1,2
    pub qos: QoSWithPacketIdentifier,
    pub payload: Arc<Vec<u8>>,
}

impl Message {
    pub fn from_pub(publish: &PublishPacket) -> Result<Box<Message>> {

        let topic = publish.topic_name().to_string();
        let topic = TopicName::new(topic).unwrap();
        // TODO From mqtt errors to rumqtt errors and do try!

        Ok(Box::new(Message {
            topic: topic,
            qos: publish.qos(),
            retain: publish.retain(),
            payload: Arc::new(publish.payload().clone()),
        }))
    }

    pub fn set_pkid(&mut self, pkid: u16) {
        match self.qos {
            QoSWithPacketIdentifier::Level0 => (),
            QoSWithPacketIdentifier::Level1(_) => self.qos = QoSWithPacketIdentifier::Level1(pkid),
            QoSWithPacketIdentifier::Level2(_) => self.qos = QoSWithPacketIdentifier::Level2(pkid),
        };
    }

    pub fn get_pkid(&self) -> Option<u16> {
        match self.qos {
            QoSWithPacketIdentifier::Level0 => None,
            QoSWithPacketIdentifier::Level1(pkid) => Some(pkid),
            QoSWithPacketIdentifier::Level2(pkid) => Some(pkid),
        }
    }

    // pub fn from_last_will(last_will: LastWill) -> Box<Message> {
    //     let topic = TopicPath::from(last_will.topic);

    //     Box::new(Message {
    //         topic: topic,
    //         qos: last_will.qos,
    //         retain: last_will.retain,
    //         pid: None,
    //         payload: Arc::new(last_will.message.into_bytes()),
    //     })
    // }

    pub fn to_pub(&self, qos: Option<QoSWithPacketIdentifier>, dup: bool) -> Box<PublishPacket> {
        let qos = qos.unwrap_or(self.qos);

        let mut publish_packet = PublishPacket::new(self.topic.clone(), qos, (&*self.payload).clone());
        publish_packet.set_dup(dup);
        publish_packet.set_retain(self.retain);

        Box::new(publish_packet)
    }

    pub fn transform(&self, qos: Option<QoSWithPacketIdentifier>) -> Box<Message> {
        let qos = qos.unwrap_or(self.qos);
        Box::new(Message {
            topic: self.topic.clone(),
            qos: qos,
            retain: self.retain,
            payload: self.payload.clone(),
        })
    }
}
