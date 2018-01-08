mod state;
mod connection;

use std::thread;
use std::sync::Arc;
use std::result::Result;
use std::mem;

use futures::sync::mpsc::{self, Sender};
use futures::{Future, Sink};
use mqtt3::*;

use MqttOptions;
use packet;

use error::ClientError;
use crossbeam_channel::{bounded, self};

/// Interface on which clients can receive messages
pub type Notification<T> = crossbeam_channel::Receiver<T>;

pub struct MqttClient {
    nw_request_tx: Option<Sender<Packet>>,
    max_packet_size: usize,
}

impl MqttClient {
    /// Connects to the broker and starts an event loop in a new thread.
    /// Returns 'Request' and handles reqests from it.
    /// Also handles network events, reconnections and retransmissions.
    pub fn start(opts: MqttOptions) -> (Self, Notification<Packet>) {
        let (commands_tx, commands_rx) = mpsc::channel(10);
        let (notifier_tx, notifier_rx) = bounded(50);

        let max_packet_size = opts.max_packet_size;

        thread::spawn( move || {
                let mut connection = connection::Connection::new(opts, notifier_tx);
                connection.start(commands_rx);
                error!("Network Thread Stopped !!!!!!!!!");
            }
        );

        let client = MqttClient { nw_request_tx: Some(commands_tx), max_packet_size: max_packet_size};
        (client, notifier_rx)
    }

    pub fn publish<S: Into<String>>(&mut self, topic: S, qos: QoS, payload: Vec<u8>) -> Result<(), ClientError>{
        let payload_len = payload.len();

        if payload_len > self.max_packet_size {
            return Err(ClientError::PacketSizeLimitExceeded)
        }

        let payload = Arc::new(payload);

        // NOTE: Don't clone 'tx' as it doubles the queue size for every clone
        let mut nw_request_tx = mem::replace(&mut self.nw_request_tx, None).unwrap();
        
        let publish = packet::gen_publish_packet(topic.into(), qos, None, false, false, payload);
        nw_request_tx = nw_request_tx.send(Packet::Publish(publish)).wait()?;

        let _ = mem::replace(&mut self.nw_request_tx, Some(nw_request_tx));
        Ok(())
    }

    // TODO: Add userdata publish

    pub fn subscribe<S: Into<String>>(&mut self, topics: Vec<(S, QoS)>) -> Result<(), ClientError> {
        if topics.len() == 0 {
            return Err(ClientError::ZeroSubscriptions);
        }

        let sub_topics: Vec<_> = topics.into_iter().map(
            |t| SubscribeTopic{topic_path: t.0.into(), qos: t.1}
        ).collect();

        // NOTE: Don't clone 'tx' as it doubles the queue size for every clone
        let mut nw_request_tx = mem::replace(&mut self.nw_request_tx, None).unwrap();

        let subscribe = Subscribe {pid: PacketIdentifier::zero(), topics: sub_topics};
        nw_request_tx = nw_request_tx.send(Packet::Subscribe(subscribe)).wait()?;
        let _ = mem::replace(&mut self.nw_request_tx, Some(nw_request_tx));
        Ok(())
    }
}
