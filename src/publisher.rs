use mio::*;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;

use error::Result;
use message::Message;
use mqtt::QualityOfService;
use mqtt::packet::*;
use mqtt::topic_name::TopicName;

use client::{MioNotification, PubNotify};

pub struct Publisher {
    pub pub0_tx: SyncSender<Message>,
    pub pub1_tx: SyncSender<Message>,
    pub pub2_tx: SyncSender<Message>,
    pub mionotify_tx: Sender<MioNotification>,
    pub retain: bool,
}

impl Publisher {
    pub fn publish(&self, topic: &str, qos: QualityOfService, payload: Vec<u8>) -> Result<()> {

        let topic = try!(TopicName::new(topic.to_string()));
        let qos_pkid = match qos {
            QualityOfService::Level0 => QoSWithPacketIdentifier::Level0,
            QualityOfService::Level1 => QoSWithPacketIdentifier::Level1(0),
            QualityOfService::Level2 => QoSWithPacketIdentifier::Level2(0),
        };

        let message = Message {
            topic: topic,
            retain: self.retain,
            qos: qos_pkid,
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

    pub fn disconnect(&self) -> Result<()> {
        try!(self.mionotify_tx.send(MioNotification::Disconnect));
        Ok(())
    }

    pub fn shutdown(&self) -> Result<()> {
        try!(self.mionotify_tx.send(MioNotification::Shutdown));
        Ok(())
    }

    pub fn set_retain(&mut self, retain: bool) -> &mut Self {
        self.retain = retain;
        self
    }
}
