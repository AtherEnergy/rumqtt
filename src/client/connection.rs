use std::net::SocketAddr;
use std::thread;
use std::cell::RefCell;
use std::rc::Rc;
use std::io::{self, ErrorKind};
use std::sync::mpsc as stdmpsc;
use std::time::Duration;

// use codec::MqttCodec;
use MqttOptions;
use client::state::MqttState;

use mqtt3::*;
// use futures::prelude::*;
use futures::stream::{Stream, SplitSink, SplitStream};
use futures::sync::mpsc::{self, Sender, Receiver};
use tokio_core::reactor::{Core, Handle};
// use tokio_core::net::TcpStream;
// use tokio_timer::Timer;
// use tokio_io::AsyncRead;
// use tokio_io::codec::Framed;

#[derive(Debug)]
pub enum Request {
    Subscribe(Vec<SubscribeTopic>),
    Publish(Publish),
    Connect,
    Ping,
    Reconnect,
}

#[derive(Debug, Clone)]
pub enum MqttRecv {
    Publish(Publish),
    Suback(Suback),
    Puback(PacketIdentifier),
}

pub fn start(opts: MqttOptions, commands_tx: Sender<Request>, commands_rx: Receiver<Request>, notifier_tx: stdmpsc::SyncSender<MqttRecv>) {

    let mut commands_rx = commands_rx.or_else(|_| {
        Err(io::Error::new(ErrorKind::Other, "Rx Error"))
    });

    // tries sends interesting incoming messages back to user
    // let notifier = notifier_tx;

    loop {
        // NOTE: If we move this out, what happen's to futures spawned in previous iteration? memory keeps growing?
        let mut reactor = Core::new().unwrap();
        let handle = reactor.handle();
        let commands_tx = commands_tx.clone();
        let notifier_tx = notifier_tx.clone();
        // TODO: fix the clone
        let opts = opts.clone();

        let mqtt_state = Rc::new(RefCell::new(MqttState::new(opts.clone())));
        let mqtt_state_connect = mqtt_state.clone();
        let mqtt_state_mqtt_recv = mqtt_state.clone();
        let mqtt_state_ping = mqtt_state.clone();

        // config
        // NOTE: make sure that dns resolution happens during reconnection incase  ip of the server changes
        // TODO: Handle all the unwraps here
        let addr: SocketAddr = opts.broker_addr.as_str().parse().unwrap();
        let reconnect_after = opts.reconnect_after.unwrap();

        
        info!("Will retry connection again in {} seconds", reconnect_after);
        thread::sleep(Duration::new(reconnect_after as u64, 0));
    }
}