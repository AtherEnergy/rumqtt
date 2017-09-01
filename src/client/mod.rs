mod connection;

use std::thread;
use std::sync::mpsc as stdmpsc;
use std::sync::Arc;
use std::time::Duration;

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

        // get 'tx' from connection to be able to send network requests to it
        let command_tx;
        loop {
            command_tx = match rx_command_tx.recv() {
                Ok(tx) => tx,
                Err(e) => {
                    info!("Waiting for connection thread for successful connection");
                    thread::sleep(Duration::new(1, 0));
                    continue;
                }
            };
            break
        }

        let client = MqttClient { nw_request_tx: command_tx,  rx_command_tx: rx_command_tx};

        client
    }

    pub fn publish(&mut self, topic: &str, qos: QoS, payload: Vec<u8>) {
        let payload = Arc::new(payload);

        loop {
            // TODO: Find ways to remove clone to improve perf
            let nw_request_tx = self.nw_request_tx.clone();

            // TODO: Fix clone
            let payload = payload.clone();
            let publish = packet::gen_publish_packet(topic, qos, None, false, false, payload);
            let r = nw_request_tx.send(NetworkRequest::Publish(publish)).wait();

            // incase of failures, fetch new 'tx' from connection and retry
            if let Ok(tx) = r {
                self.nw_request_tx = tx;
                break;
            } else {
                loop {
                    match self.rx_command_tx.recv() {
                        Ok(tx) => {
                            self.nw_request_tx = tx;
                            break;
                        }
                        Err(e) => {
                            info!("Waiting for connection thread for successful connection");
                            thread::sleep(Duration::new(1, 0));
                            continue;
                        }
                    };
                }
            }
        }
    }
}