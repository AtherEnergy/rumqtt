mod connection;

use std::thread;
use std::sync::mpsc as stdmpsc;
use std::sync::Arc;

use futures::sync::mpsc::Sender;
use futures::{Future, Sink};
use mqtt3::*;

use MqttOptions;
use packet;

use self::connection::NetworkRequest;

pub struct MqttClient {
    nw_request_tx: Sender<NetworkRequest>,
    rx_command_tx: stdmpsc::Receiver<Sender<NetworkRequest>>,
}

impl MqttClient {
    /// Connects to the broker and starts an event loop in a new thread.
    /// Returns 'Request' and handles reqests from it.
    /// Also handles network events, reconnections and retransmissions.
    pub fn start(opts: MqttOptions) -> Self {
        let (tx_command_tx, rx_command_tx) = stdmpsc::sync_channel(10);

        // This thread handles network reads (coz they are blocking) and
        // and sends them to event loop thread to handle mqtt state.
        thread::spawn( move || {
                connection::start(opts, tx_command_tx);
                error!("Network Thread Stopped !!!!!!!!!");
            }
        );

        let command_tx = rx_command_tx.recv().unwrap();

        let client = MqttClient { nw_request_tx: command_tx,  rx_command_tx: rx_command_tx};

        client
    }

    pub fn publish(&mut self, topic: &str, qos: QoS, payload: Vec<u8>) {
        let payload = Arc::new(payload);

        // TODO: Find ways to remove clone to improve perf
        let mut nw_request_tx = self.nw_request_tx.clone();

        loop {
            let payload = payload.clone();
            let publish = packet::gen_publish_packet(topic, qos, None, false, false, payload);

            let r = nw_request_tx.send(NetworkRequest::Publish(publish)).wait();

            nw_request_tx = match r {
                Ok(tx) => tx,
                Err(e) => self.rx_command_tx.recv().unwrap()
            };
        }
    }
}