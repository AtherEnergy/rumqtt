mod connection;

use std::thread;
use std::sync::Arc;
use std::time::Duration;

use futures::sync::mpsc::{self, Sender};
use futures::{Future, Sink};
use mqtt3::*;

use MqttOptions;
use packet;

use self::connection::Request;

pub struct MqttClient {
    nw_request_tx: Sender<Request>,
}

impl MqttClient {
    /// Connects to the broker and starts an event loop in a new thread.
    /// Returns 'Request' and handles reqests from it.
    /// Also handles network events, reconnections and retransmissions.
    pub fn start(opts: MqttOptions) -> Self {
        let (mut commands_tx, commands_rx) = mpsc::channel(10);
        let nw_commands_tx = commands_tx.clone();

        // This thread handles network reads (coz they are blocking) and
        // and sends them to event loop thread to handle mqtt state.
        thread::spawn( move || {
                connection::start(opts, nw_commands_tx, commands_rx);
                error!("Network Thread Stopped !!!!!!!!!");
            }
        );

        commands_tx = commands_tx.send(Request::Connect).wait().unwrap();
        let client = MqttClient { nw_request_tx: commands_tx};
        client
    }

    pub fn publish(&mut self, topic: &str, qos: QoS, payload: Vec<u8>) {
        let payload = Arc::new(payload);

        // TODO: Find ways to remove clone to improve perf
        let nw_request_tx = self.nw_request_tx.clone();
        // TODO: Fix clone
        let payload = payload.clone();
        let publish = packet::gen_publish_packet(topic, qos, None, false, false, payload);
        let r = nw_request_tx.send(Request::Publish(publish)).wait();
    }
}