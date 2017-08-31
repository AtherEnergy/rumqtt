use std::sync::mpsc as stdmpsc;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use std::collections::VecDeque;

use futures::prelude::*;
use futures::stream::{Stream, SplitSink};
use futures::sync::mpsc::{self, Sender, Receiver};
use tokio_core::reactor::Core;
use tokio_core::net::TcpStream;
use tokio_timer::Timer;
use tokio_io::AsyncRead;
use tokio_io::codec::Framed;
use mqtt3::*;
use threadpool::ThreadPool;

use codec::MqttCodec;
use MqttOptions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MqttConnectionState {
    Handshake,
    Connected,
    Disconnected,
}

struct MqttState {
    addr: SocketAddr,
    opts: MqttOptions,

    // --------  State  ----------
    connection_state: MqttConnectionState,
    initial_connect: bool,
    await_pingresp: bool,
    last_flush: Instant,
    last_pkid: PacketIdentifier,

    // For QoS 1. Stores outgoing publishes
    outgoing_pub: VecDeque<Message>,
    // clean_session=false will remember subscriptions only till lives.
    // If broker crashes, all its state will be lost (most brokers).
    // client wouldn't want to loose messages after it comes back up again
    subscriptions: VecDeque<SubscribeTopic>,

    // --------  Callbacks  --------
    // callback: Option<MqttCallback>,

    // -------- Thread pool to execute callbacks
    pool: ThreadPool,
}



#[cfg(test)]
mod test {
}