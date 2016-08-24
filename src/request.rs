use mio::*;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;

use error::Result;
use message::Message;
use mqtt::{QualityOfService, TopicFilter};
use mqtt::packet::*;
use mqtt::topic_name::TopicName;

use client::{MioNotification, PubNotify};

pub struct MqRequest {
    pub pub0_tx: SyncSender<Message>,
    pub pub1_tx: SyncSender<Message>,
    pub pub2_tx: SyncSender<Message>,
    pub subscribe_tx: SyncSender<Vec<(TopicFilter, QualityOfService)>>,
    pub mionotify_tx: Sender<MioNotification>,
}

impl MqRequest {
    pub fn publish(&self, topic: &str, retain: bool, qos: QualityOfService, payload: Vec<u8>) -> Result<()> {

        let topic = try!(TopicName::new(topic.to_string()));
        let qos_pkid = match qos {
            QualityOfService::Level0 => QoSWithPacketIdentifier::Level0,
            QualityOfService::Level1 => QoSWithPacketIdentifier::Level1(0),
            QualityOfService::Level2 => QoSWithPacketIdentifier::Level2(0),
        };

        // TODO: Why are qos and pkid in the same structure
        let message = Message {
            topic: topic,
            retain: retain,
            qos: qos_pkid,
            // Optimizes clones
            payload: Arc::new(payload),
        };

        // TODO: Check message sanity here and return error if not
        match qos {
            QualityOfService::Level0 => {
                try!(self.pub0_tx.send(message));
                try!(self.mionotify_tx.send(MioNotification::Pub(PubNotify::QoS0)));
            }
            QualityOfService::Level1 => {
                // Order important coz mioco is level triggered
                try!(self.pub1_tx.send(message));
                try!(self.mionotify_tx.send(MioNotification::Pub(PubNotify::QoS1)));
            }
            QualityOfService::Level2 => {
                try!(self.pub2_tx.send(message));
                try!(self.mionotify_tx.send(MioNotification::Pub(PubNotify::QoS2)));
            }
        };

        Ok(())
    }

    pub fn subscribe(&self, topics: Vec<(&str, QualityOfService)>) -> Result<()> {
        let mut sub_topics = vec![];
        for topic in topics {
            let topic = (try!(TopicFilter::new_checked(topic.0)), topic.1);
            sub_topics.push(topic);
        }

        try!(self.subscribe_tx.send(sub_topics));
        try!(self.mionotify_tx.send(MioNotification::Sub));
        Ok(())
    }

    pub fn disconnect(&self) -> Result<()> {
        try!(self.mionotify_tx.send(MioNotification::Disconnect));
        Ok(())
    }

    pub fn shutdown(&self) -> Result<()> {
        try!(self.mionotify_tx.send(MioNotification::Shutdown));
        Ok(())
    }
}
