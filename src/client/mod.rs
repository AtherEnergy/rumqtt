use std::sync::Arc;
use mqtt3::{Publish, PacketIdentifier, QoS};
use futures::sync::mpsc;
use futures::{Future, Sink};
use crossbeam_channel;
use error::ClientError;
use MqttOptions;
use mqtt3::Subscribe;
use mqtt3::SubscribeTopic;

pub mod connection;
pub mod mqttstate;
pub mod network;

#[derive(Debug)]
pub enum Notification {
    Publish(Arc<Vec<u8>>),
    PubAck(u16),
    PubRec(u16),
    PubRel(u16),
    PubComp(u16),
    SubAck(u16),
    None
}

/// Requests to network event loop
pub enum Request {
    Publish(Publish),
    Subscribe(Subscribe),
    PubAck(PacketIdentifier),
    Ping,
    Reconnect(MqttOptions),
    Disconnect,
    None
}

pub struct MqttClient {
    userrequest_tx: mpsc::Sender<Request>,
    max_packet_size: usize
}

impl MqttClient {
    pub fn start(opts: MqttOptions) -> (Self, crossbeam_channel::Receiver<Notification>) {
         let (userrequest_tx, notification_rx) = connection::Connection::run(opts);

        //TODO: Remove max packet size hardcode
        let client = MqttClient {
             userrequest_tx,
             max_packet_size: 1000
        };

        (client, notification_rx)
    }

    pub fn publish<S: Into<String>, V: Into<Vec<u8>>>(&mut self, topic: S, qos: QoS, payload: V) -> Result<(), ClientError> {
        let payload = payload.into();
        if payload.len() > self.max_packet_size {
            return Err(ClientError::PacketSizeLimitExceeded)
        }

        //TODO: Rename `pid` to `pkid` in mqtt311
        let publish =  Publish {
            dup: false,
            qos,
            retain: false,
            topic_name: topic.into(),
            pid: None,
            payload: Arc::new(payload),
        };

        let tx = &mut self.userrequest_tx;
        tx.send(Request::Publish(publish)).wait()?;
        Ok(())
    }

    pub fn subscribe<S: Into<String>>(&mut self, topic: S, qos: QoS) -> Result<(), ClientError> {
        let topic = SubscribeTopic{topic_path: topic.into(), qos: qos};
        let subscribe = Subscribe {pid: PacketIdentifier::zero(), topics: vec![topic]};

        let tx = &mut self.userrequest_tx;
        tx.send(Request::Subscribe(subscribe)).wait()?;
        Ok(())
    }

    pub fn disconnect(&mut self) -> Result<(), ClientError> {
        let tx = &mut self.userrequest_tx;
        tx.send(Request::Disconnect).wait()?;
        Ok(())
    }
}
