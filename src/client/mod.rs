mod connection;

use std::thread;

use futures::sync::mpsc::Sender;
use std::sync::mpsc as stdmpsc;

use MqttOptions;

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
}