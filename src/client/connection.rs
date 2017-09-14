use std::net::SocketAddr;
use std::thread;
use std::cell::RefCell;
use std::rc::Rc;
use std::io::{self, ErrorKind};
use std::sync::mpsc as stdmpsc;
use std::time::Duration;
use std::error::Error;
use std::collections::VecDeque;

use codec::MqttCodec;
use MqttOptions;
use client::state::MqttState;

use mqtt3::*;
use futures::stream::{Stream, SplitSink, SplitStream};
use futures::sync::mpsc::{Sender, Receiver};
use tokio_core::reactor::Core;
use futures::prelude::*;

use tokio_core::net::TcpStream;
use tokio_timer::Timer;
use tokio_io::AsyncRead;
use tokio_io::codec::Framed;

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
        // TODO: Handle all the unwraps here
        let reconnect_after = opts.reconnect_after.unwrap();

        let framed = match mqtt_connect(mqtt_state_connect, opts.clone(), &mut reactor) {
            Ok(framed) => framed,
            Err(e) => {
                error!("Connection error = {:?}", e);
                continue;
            }
        };

        let mut last_session_publishes = mqtt_state.borrow_mut().handle_reconnection();

        let client = async_block! {
            let (mut sender, receiver) = framed.split();
            let ping_commands_tx = commands_tx.clone();
            let nw_commands_tx = commands_tx.clone();

            while let Some(publish) = last_session_publishes.pop_front() {
                let publish = mqtt_state.borrow_mut().handle_outgoing_publish(publish);

                if let Err(e) = publish {
                    return Err(io::Error::new(ErrorKind::Other, e.description()));
                }

                let packet = Packet::Publish(publish.unwrap());
                // Ok to block event loop during reconnection
                // TODO: add await when references are supported
                sender = sender.send(packet).wait().unwrap();
            }

            // incoming network messages
            handle.spawn(
                mqtt_recv(mqtt_state_mqtt_recv, receiver, nw_commands_tx).then(|result| {

                match result {
                    Ok(_) => error!("N/w receiver done"),
                    Err(e) => error!("N/w IO error {:?}", e),
                }
                Ok(())
            }));

            // ping timer
            handle.spawn(
                ping_timer(mqtt_state_ping, ping_commands_tx, opts.keep_alive.unwrap()).then(|result| {
                match result {
                    Ok(_) => error!("Ping timer done"),
                    Err(e) => error!("Ping timer IO error {:?}", e),
                }
                Ok(())
            }));

            // execute user requests  
            loop {
                let command = match await!(commands_rx.into_future().map_err(|e| e.0))? {
                    (Some(item), s) => {
                        commands_rx = s;
                        item
                    }
                    (None, s) => {
                        commands_rx = s;
                        break
                    }
                };

                info!("command = {:?}", command);

                let packet = match command {
                    Request::Publish(publish) => {
                        // BUG(generators): https://github.com/rust-lang/rust/issues/44184
                        let publish = publish;
                        let publish = mqtt_state.borrow_mut().handle_outgoing_publish(publish);

                        if let Err(e) = publish {
                            return Err(io::Error::new(ErrorKind::Other, e.description()));
                        }

                        Packet::Publish(publish.unwrap())
                    },
                    Request::Ping => {
                        let ping = mqtt_state.borrow_mut().handle_outgoing_ping();
                        if let Err(e) = ping {
                            return Err(io::Error::new(ErrorKind::Other, e.description()));
                        }
                        
                        Packet::Pingreq
                    }
                    _ => unimplemented!(),
                };

                sender = await!(sender.send(packet))?
            } // end of command recv loop

            error!("Done with network receiver !!");
            Ok::<_, io::Error>(commands_rx)
        }; // end of async mqtt future

        let response = reactor.run(client);
        commands_rx = response.unwrap();

        info!("Will retry connection again in {} seconds", reconnect_after);
        thread::sleep(Duration::new(reconnect_after as u64, 0));
    }
}

// DESIGN: Initial connect status should be immediately known.
//         Intermediate disconnections should be automatically reconnected
fn mqtt_connect(mqtt_state: Rc<RefCell<MqttState>>, opts: MqttOptions, reactor: &mut Core) -> io::Result<Framed<TcpStream, MqttCodec>> {
    // NOTE: make sure that dns resolution happens during reconnection to handle changes in server ip
    let addr: SocketAddr = opts.broker_addr.as_str().parse().unwrap();

    let f_response = TcpStream::connect(&addr, &reactor.handle()).and_then(|connection| {
        let framed = connection.framed(MqttCodec);
        let connect = mqtt_state.borrow_mut().handle_outgoing_connect();
        let f1 = framed.send(Packet::Connect(connect));

        f1.and_then(|framed| {
            framed.into_future().and_then(|(res, stream)| Ok((res, stream))).map_err(|(err, _stream)| err)
        })
    });

    let response = reactor.run(f_response);
    
    let (packet, frame) = response?;

    // Return `Framed` and previous session packets that are to be republished
    match packet.unwrap() {
        Packet::Connack(connack) => {
            let mqtt_connect_response = mqtt_state.borrow_mut().handle_incoming_connack(connack);
            if let Err(e) = mqtt_connect_response {
                Err(io::Error::new(ErrorKind::Other, e.description()))
            } else {
                let previous_session_packets = mqtt_connect_response.unwrap();
                Ok(frame)
            }
        }
        _ => unimplemented!(),
    }
}

#[async]
fn ping_timer(mqtt_state: Rc<RefCell<MqttState>>, mut commands_tx: Sender<Request>, keep_alive: u16) -> io::Result<()> {
    let timer = Timer::default();
    let interval = timer.interval(Duration::new(keep_alive as u64, 0));

    #[async]
    for _t in interval {
        if mqtt_state.borrow().is_ping_required() {
            debug!("Ping timer fire");
            commands_tx = await!(
                commands_tx.send(Request::Ping).or_else(|e| {
                    Err(io::Error::new(ErrorKind::Other, e.description()))
                })
            )?;
        }
    }

    Ok(())
}

#[async]
fn mqtt_recv(mqtt_state: Rc<RefCell<MqttState>>, receiver: SplitStream<Framed<TcpStream, MqttCodec>>, commands_tx: Sender<Request>) -> io::Result<()> {

    #[async]
    for message in receiver {
        info!("incoming n/w message = {:?}", message);
        match message {
            Packet::Connack(connack) => {
                // TODO: Handle result
                let _ = mqtt_state.borrow_mut().handle_incoming_connack(connack);
            }
            Packet::Puback(ack) => {
                let _ = mqtt_state.borrow_mut().handle_incoming_puback(ack);
            }
            Packet::Pingresp => {
                let _ = mqtt_state.borrow_mut().handle_incoming_pingresp();
            }
            _ => unimplemented!()
        }
    }

    error!("Network reciever stopped. Sending reconnect request");
    await!(commands_tx.send(Request::Reconnect));
    Ok(())
}