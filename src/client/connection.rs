use std::cell::RefCell;
use std::rc::Rc;
use std::net::SocketAddr;
use std::time::Duration;
use std::thread;
use std::sync::mpsc as stdmpsc;

use futures::{future, Future, Sink};
use futures::stream::{Stream, SplitStream};
use futures::sync::mpsc::{Sender, Receiver};
use tokio_core::reactor::Core;
use tokio_core::net::TcpStream;
use tokio_timer::Timer;
use tokio_io::AsyncRead;
use tokio_io::codec::Framed;

use mqtt3::Packet;

 use std::io::{Error, ErrorKind};

use error::*;
use mqttopts::{MqttOptions, ReconnectOptions};
use client::Request;
use client::state::MqttState;
use codec::MqttCodec;

pub struct Connection {
    notifier_tx: stdmpsc::SyncSender<Packet>,
    commands_tx: Sender<Request>,

    mqtt_state: Rc<RefCell<MqttState>>,
    opts: MqttOptions,
    reactor: Core,
}

impl Connection {
    pub fn new(opts: MqttOptions, commands_tx: Sender<Request>, notifier_tx: stdmpsc::SyncSender<Packet>) -> Self {
        Connection {
            notifier_tx: notifier_tx,
            commands_tx: commands_tx,
            mqtt_state: Rc::new(RefCell::new(MqttState::new(opts.clone()))),
            opts: opts,
            reactor: Core::new().unwrap()
        }
    }

    pub fn start(&mut self, mut commands_rx: Receiver<Request>) {
        let initial_connect = self.mqtt_state.borrow().initial_connect();
        let reconnect_opts = self.opts.reconnect;

        'reconnect: loop {
            let framed = match self.mqtt_connect() {
                Ok(framed) => framed,
                Err(e) => {
                    error!("Connection error = {:?}", e);
                    match reconnect_opts {
                        ReconnectOptions::Never => break 'reconnect,
                        ReconnectOptions::AfterFirstSuccess(d) if !initial_connect => {
                            info!("Will retry connecting again in {} seconds", d);
                            thread::sleep(Duration::new(u64::from(d), 0));
                            continue 'reconnect;
                        }
                        ReconnectOptions::AfterFirstSuccess(_) => break 'reconnect,
                        ReconnectOptions::Always(d) => {
                            info!("Will retry connecting again in {} seconds", d);
                            thread::sleep(Duration::new(u64::from(d), 0));
                            continue 'reconnect;
                        }
                    }
                }
            };

            let (mut sender, receiver) = framed.split();
            let mut commands_tx = self.commands_tx.clone();
            let mut disconnect_tx = self.commands_tx.clone();
            let receiver = receiver.then(|result| {
                let commands_tx = &mut commands_tx;
                let message = match result {
                    Ok(m) => {
                        println!("Received {:?}", m);
                        m
                    },
                    Err(e) => {
                        error!("Network receiver error = {:?}", e);
                        commands_tx.send(Request::Disconnect).wait().unwrap();
                        return future::err(e)
                    }
                };
                future::ok(())
            }).for_each(|_| future::ok(()));

            let receiver = receiver.then(|result| {
                // This returns when the stream is done or has errored out
                println!("Outer result {:?}", result);
                match result {
                    Ok(v) => {
                        error!("Network receiver done!!. Result = {:?}", v);
                        // disconnect_tx.send(Request::Disconnect).wait().unwrap();
                        // its the end of stream or either an error, so we return an error
                        // to jump back to reconnect loop
                        return future::err(())
                    }
                    Err(e) => error!("N/w receiver failed. Error = {:?}", e),
                }
                future::ok(())
            });

            // spawn ping timer
            if let Some(keep_alive) = self.opts.keep_alive {
                self.spawn_ping_timer(keep_alive);
            }

            // republish last session unacked packets
            let last_session_publishes = self.mqtt_state.borrow_mut().handle_reconnection();
            if last_session_publishes.is_some() {
                for publish in last_session_publishes.unwrap() {
                    let packet = Packet::Publish(publish);
                    sender = sender.send(packet).wait().unwrap();
                }
            }

            // receive incoming user request and write to network
            // TODO spawn this as a future
            let mqtt_state = self.mqtt_state.clone();
            let cmd_rx = commands_rx.by_ref();

            let pack_gen = |msg| {
               let packet = mqtt_state.borrow_mut().handle_client_requests(msg).unwrap();
               return packet
            };

            let client_to_tcp_sender = cmd_rx.map(|msg| pack_gen(msg))
                                             .map_err(|_| Error::new(ErrorKind::Other, "Error receiving client msg"))
                                             .forward(sender)
                                             // convert the tuple type to ()
                                             .map(|_| ())
                                             // same for error
                                             .or_else(|_| { println!("Client send error"); future::ok(())});

            let fused_future = receiver.select(client_to_tcp_sender);

            if let Err(e) = self.reactor.run(fused_future) {
                error!("Reactor halted. Error")
            }
        }
    }


    fn mqtt_connect(&mut self) -> Result<Framed<TcpStream, MqttCodec>, ConnectError> {
        // NOTE: make sure that dns resolution happens during reconnection to handle changes in server ip
        let addr: SocketAddr = self.opts.broker_addr.as_str().parse().unwrap();
        let handle = self.reactor.handle();
        let mqtt_state = self.mqtt_state.clone();
        
        // TODO: Add TLS support with client authentication (ca = roots.pem for iotcore)

        let future_response = TcpStream::connect(&addr, &handle).and_then(|connection| {
            let framed = connection.framed(MqttCodec);
            let connect = mqtt_state.borrow_mut().handle_outgoing_connect();
            let future_mqtt_connect = framed.send(Packet::Connect(connect));

            future_mqtt_connect.and_then(|framed| {
                framed.into_future().and_then(|(res, stream)| Ok((res, stream))).map_err(|(err, _stream)| err)
            })
        });

        let response = self.reactor.run(future_response);
        let (packet, frame) = response?;

        // Return `Framed` and previous session packets that are to be republished
        match packet.unwrap() {
            Packet::Connack(connack) => {
                self.mqtt_state.borrow_mut().handle_incoming_connack(connack)?;
                Ok(frame)
            }
            _ => unimplemented!(),
        }
    }

    fn spawn_ping_timer(&self, keep_alive: u16) {
        let timer = Timer::default();
        let interval = timer.interval(Duration::new(u64::from(keep_alive), 0));
        let mqtt_state = self.mqtt_state.clone();
        let mut commands_tx = self.commands_tx.clone();
        let handle = self.reactor.handle();
        let timer_future = interval.for_each(move |_t| {
            let ref mut commands_tx = commands_tx;
            if mqtt_state.borrow().is_ping_required() {
                debug!("Ping timer fire");
                commands_tx.send(Request::Ping).wait().unwrap();
            }
            future::ok(())
        });

        handle.spawn(
            timer_future.then(move |result| {
                    match result {
                        Ok(_) => error!("Ping timer done"),
                        Err(e) => error!("Ping timer IO error {:?}", e),
                    }
                    future::ok(())
                }
            )
        )
    }
}