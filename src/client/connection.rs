use std::cell::RefCell;
use std::rc::Rc;
use std::net::SocketAddr;
use std::time::Duration;

use futures::{future, Future, Sink};
use futures::stream::{Stream, SplitStream};
use futures::sync::mpsc::{Sender, Receiver};
use tokio_core::reactor::Core;
use tokio_core::net::TcpStream;
use tokio_timer::Timer;
use tokio_io::AsyncRead;
use tokio_io::codec::Framed;

use failure::Error;
use mqtt3::Packet;

use error::*;
use mqttopts::MqttOptions;
use client::Request;
use client::state::MqttState;
use codec::MqttCodec;

pub struct Connection {
    notifier_tx: ::std::sync::mpsc::SyncSender<Packet>,
    commands_tx: Sender<Request>,
    commands_rx: Receiver<Request>,

    mqtt_state: Rc<RefCell<MqttState>>,
    opts: MqttOptions,
    reactor: Core,
}

impl Connection {
    pub fn start() {

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

    fn handle_client_requests(&self, client_request: Request) -> Result<Packet, Error> {
        match client_request {
            Request::Publish(publish) => {
                let publish = self.mqtt_state.borrow_mut().handle_outgoing_publish(publish)?;
                Ok(Packet::Publish(publish))
            },
            Request::Ping => {
                let _ping = self.mqtt_state.borrow_mut().handle_outgoing_ping()?;
                Ok(Packet::Pingreq)
            }
            Request::Subscribe(subs) => {
                let subscription = self.mqtt_state.borrow_mut().handle_outgoing_subscribe(subs)?;
                Ok(Packet::Subscribe(subscription))
            }
            Request::Disconnect => {
                self.mqtt_state.borrow_mut().handle_disconnect();
                Ok(Packet::Disconnect)
            },
            Request::Puback(pkid) => Ok(Packet::Puback(pkid)),
            _ => unimplemented!(),
        }
    }

    fn spawn_ping_timer(&self, keep_alive: u16) -> Result<(), Error> {
        let timer = Timer::default();
        let interval = timer.interval(Duration::new(u64::from(keep_alive), 0));
        let mqtt_state = self.mqtt_state.clone();
        let mut commands_tx = self.commands_tx.clone();
        let handle = self.reactor.handle();

        let timer_future = interval.for_each(move |t| {
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
        );

        Ok(())
    }

    fn spawn_incoming_network_packet_handler(&self, receiver: SplitStream<Framed<TcpStream, MqttCodec>>) -> Result<(), Error> {
        let mqtt_state = self.mqtt_state.clone();
        let mut commands_tx = self.commands_tx.clone();
        let notifier = self.notifier_tx.clone();
        let handle = self.reactor.handle();

        let receiver = receiver.for_each(move |message| {
                let ref mut commands_tx = commands_tx;
                match message {
                Packet::Connack(connack) => {
                    if let Err(e) = mqtt_state.borrow_mut().handle_incoming_connack(connack) {
                        error!("Connack failed. Error = {:?}", e);
                    }
                }
                Packet::Puback(ack) => {
                    if let Err(e) = notifier.try_send(Packet::Puback(ack)) {
                        error!("Puback notification send failed. Error = {:?}", e);
                    }
                    // ignore unsolicited ack errors
                    let _ = mqtt_state.borrow_mut().handle_incoming_puback(ack);
                }
                Packet::Pingresp => {
                    mqtt_state.borrow_mut().handle_incoming_pingresp();
                }
                Packet::Publish(publish) => {
                    let (publish, ack) = mqtt_state.borrow_mut().handle_incoming_publish(publish);
                    if let Some(publish) = publish {
                        if let Err(e) = notifier.try_send(Packet::Publish(publish)) {
                            error!("Publish notification send failed. Error = {:?}", e);
                        }
                    }
                    if let Some(ack) = ack {
                        match ack {
                            Packet::Puback(pkid) => {
                                commands_tx.send(Request::Puback(pkid)).wait().unwrap();
                            }
                            _ => unimplemented!()
                        };
                    }
                }
                Packet::Suback(suback) => {
                    if let Err(e) = notifier.try_send(Packet::Suback(suback)) {
                        error!("Suback notification send failed. Error = {:?}", e);
                    }
                }
                _ => unimplemented!()
            }

            commands_tx.send(Request::Disconnect).wait().unwrap();
            future::ok(())
        });

        handle.spawn(
            receiver.then(move |result| {
                match result {
                    Ok(_) => error!("Network receiver done!!"),
                    Err(e) => error!("N/w receiver failed. Error = {:?}", e),
                }

                future::ok(())
            })
        );
        
        Ok(())
    }
}