use std::sync::Arc;
use std::sync::mpsc;
use mio::channel::SyncSender;

use error::Result;
use message::Message;
use mqtt::{QualityOfService, TopicFilter};
use mqtt::packet::*;
use mqtt::topic_name::TopicName;

use client::MioNotification;

pub enum StatsReq {
    QoS1QLen,
    QoS2QLen,
}

pub enum StatsResp {
    QoS1QLen(usize),
    QoS2QLen(usize),
}

pub struct MqRequest {
    // QoS 0 publish request to eventloop
    pub pub0_tx: SyncSender<Message>,
    // QoS 1 publish request to eventloop
    pub pub1_tx: SyncSender<Message>,
    // QoS 2 publish request to eventloop
    pub pub2_tx: SyncSender<Message>,
    // Subscribe request to eventloop
    pub subscribe_tx: SyncSender<Vec<(TopicFilter, QualityOfService)>>,
    // miscellaneous requests to eventloop
    pub misc_tx: SyncSender<MioNotification>,
    pub stats_req_tx: SyncSender<StatsReq>,
    pub stats_resp_rx: mpsc::Receiver<StatsResp>,
}

impl MqRequest {
    pub fn publish(&self, topic: &str, qos: QualityOfService, payload: Vec<u8>) -> Result<()> {
        self._publish(topic, false, qos, payload, None)
    }

    pub fn retained_publish(&self, topic: &str, qos: QualityOfService, payload: Vec<u8>) -> Result<()> {
        self._publish(topic, true, qos, payload, None)
    }

    pub fn userdata_publish(&self, topic: &str, qos: QualityOfService, payload: Vec<u8>, userdata: Vec<u8>) -> Result<()> {
        self._publish(topic, false, qos, payload, Some(userdata))
    }

    pub fn retained_userdata_publish(&self,
                                     topic: &str,
                                     qos: QualityOfService,
                                     payload: Vec<u8>,
                                     userdata: Vec<u8>)
                                     -> Result<()> {
        self._publish(topic, true, qos, payload, Some(userdata))
    }

    fn _publish(&self,
                topic: &str,
                retain: bool,
                qos: QualityOfService,
                payload: Vec<u8>,
                userdata: Option<Vec<u8>>)
                -> Result<()> {

        let topic = try!(TopicName::new(topic.to_string()));
        let qos_pkid = match qos {
            QualityOfService::Level0 => QoSWithPacketIdentifier::Level0,
            QualityOfService::Level1 => QoSWithPacketIdentifier::Level1(0),
            QualityOfService::Level2 => QoSWithPacketIdentifier::Level2(0),
        };

        // TODO: use a combinator instead
        let userdata = match userdata {
            Some(u) => Some(Arc::new(u)),
            None => None,
        };

        // TODO: Why are qos and pkid in the same structure
        let message = Message {
            topic: topic,
            retain: retain,
            qos: qos_pkid,
            // Optimizes clones
            payload: Arc::new(payload),
            userdata: userdata,
        };

        // TODO: Check message sanity here and return error if not
        match qos {
            QualityOfService::Level0 => {
                try!(self.pub0_tx.send(message));
            }
            QualityOfService::Level1 => {
                // Order important coz mioco is level triggered
                try!(self.pub1_tx.send(message));
            }
            QualityOfService::Level2 => {
                try!(self.pub2_tx.send(message));
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
        Ok(())
    }

    pub fn disconnect(&self) -> Result<()> {
        try!(self.misc_tx.send(MioNotification::Disconnect));
        Ok(())
    }

    pub fn shutdown(&self) -> Result<()> {
        try!(self.misc_tx.send(MioNotification::Shutdown));
        Ok(())
    }

    pub fn qos1_q_len(&self) -> Result<usize> {
        try!(self.stats_req_tx.send(StatsReq::QoS1QLen));
        let o = try!(self.stats_resp_rx.recv());
        if let StatsResp::QoS1QLen(r) = o {
            return Ok(r);
        } else {
            panic!("Invalid Stats Response");
        }
    }

    pub fn qos2_q_len(&self) -> Result<usize> {
        try!(self.stats_req_tx.send(StatsReq::QoS2QLen));
        let o = try!(self.stats_resp_rx.recv());
        if let StatsResp::QoS2QLen(r) = o {
            return Ok(r);
        } else {
            panic!("Invalid Stats Response");
        }
    }
}
