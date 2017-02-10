use std::time::{Duration, Instant};
use std::collections::VecDeque;
use std::sync::Arc;
use std::net::{SocketAddr, ToSocketAddrs};
use std::cell::RefCell;

use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;
use tokio_core::io::{Io, Framed};
use tokio_timer::{Timer, Interval};
use futures::Future;
use futures::Sink;
use futures::Stream;
use futures::sync::mpsc;

use mqtt3::{Message, QoS, TopicPath, PacketIdentifier, SubscribeTopic, Packet};

use error::*;
use packet;
use codec::MqttCodec;
use clientoptions::MqttOptions;

pub type MessageSendableFn = Box<Fn(Message) + Send + Sync>;
pub type PublishSendableFn = Box<Fn(Message) + Send + Sync>;

pub struct Connection {
    /// Holds connection state
    pub connection_state: MqttState,
    /// Holds various options according to client prefs
    pub opts: MqttOptions,
    /// Whether its an initial reconnnect
    pub initial_connect: bool,
    pub last_flush: Instant,

    /// Number of reconnections that happened
    pub no_of_reconnections: u32,

    /// `State` holds data that needs mutation
    pub state: RefCell<State>,

    /// On message callback
    pub message_callback: Option<Arc<MessageSendableFn>>,
    /// On publish callback
    pub publish_callback: Option<Arc<PublishSendableFn>>,

    // clean_session=false will remember subscriptions only till lives.
    // If broker crashes, all its state will be lost (most brokers).
    // client wouldn't want to loose messages after it comes back up again
    pub subscriptions: VecDeque<Vec<SubscribeTopic>>, // pub reactor: Core,
}

/// States which needs mutation. We keep state as a seperate struct to avoid compile time borrow checker errors.
pub struct State {
    /// Flag which describes the ping response ack state, and is initially true.
    last_pingresp: bool,
    // Queues. Note: 'record' is qos2 term for 'publish'
    /// For QoS 1. Stores outgoing publishes
    pub outgoing_pub: VecDeque<Message>,
    /// For QoS 2. Store for incoming publishes to record.
    pub incoming_rec: VecDeque<Message>, //
    /// For QoS 2. Store for outgoing publishes.
    pub outgoing_rec: VecDeque<Message>,
    /// For Qos2. Store for outgoing `pubrel` packets.
    pub outgoing_rel: VecDeque<PacketIdentifier>,
    /// For Qos2. Store for outgoing `pubcomp` packets.
    pub outgoing_comp: VecDeque<PacketIdentifier>,
}

impl Default for State {
    fn default() -> Self {
        State {
            last_pingresp: true,
            outgoing_pub: VecDeque::new(),
            incoming_rec: VecDeque::new(),
            outgoing_rec: VecDeque::new(),
            outgoing_rel: VecDeque::new(),
            outgoing_comp: VecDeque::new()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttState {
    Handshake,
    Connected,
    Disconnected,
}

#[derive(Debug)]
pub enum NetworkRequest {
    Subscribe(Vec<(TopicPath, QoS)>),
    Publish(Message),
    Retransmit,
    Shutdown,
    Disconnect,
    Ping,
}

#[derive(Debug)]
pub enum NetworkNotification {
    Disconnected,
    Connected,
}

// DESIGN: Initial connect status should be immediately known.
//        Intermediate disconnections should be automatically reconnected
fn try_reconnect(opts: MqttOptions,
                 reactor: &mut Core)
                 -> Result<Framed<TcpStream, MqttCodec>, Error> {
    let addr: SocketAddr = opts.get_ip_addr().parse()?;
    let connect_packet = packet::gen_connect_packet(opts, None, None);
    let f_response = TcpStream::connect(&addr, &reactor.handle()).and_then(|connection| {
        let framed = connection.framed(MqttCodec);
        let f1 = framed.send(connect_packet);

        f1.and_then(|framed| {
                framed.into_future()
                    .and_then(|(res, stream)| Ok((res, stream)))
                    .map_err(|(err, _stream)| err)
            })
            .boxed()
    });

    let response = reactor.run(f_response);
    // TODO: Check ConnAck Status and Error out incase of failure
    let (packet, frame) = response?;
    Ok(frame)
}

impl Connection {
    pub fn start(opts: MqttOptions,
                 publish_callback: Option<Arc<PublishSendableFn>>,
                 message_callback: Option<Arc<MessageSendableFn>>)
                 -> Result<Self, Error> {

        let connection = Connection {
            connection_state: MqttState::Disconnected,
            opts: opts,
            initial_connect: true,
            last_flush: Instant::now(),
            state: RefCell::new(Default::default()),
            no_of_reconnections: 0,
            publish_callback: publish_callback,
            message_callback: message_callback,

            // Subscriptions
            subscriptions: VecDeque::new(),
        };

        Ok(connection)
    }

    /// Returns an Internal which keeps ticking over `keep_alive` secs.
    fn pingtimer(&self) -> Interval {
        let timer = Timer::default();

        let keep_alive = if self.opts.keep_alive > 0 {
            self.opts.keep_alive as u64
        } else {
            1000
        };

        timer.interval(Duration::new(keep_alive, 0))
    }

    /// Spin the event loop of the mqtt client.
    pub fn run(&mut self) -> Result<(), Error> {
        let mut reactor = Core::new()?;

        'reconnect: loop {
            let framed;
            loop {
                if self.initial_connect {
                    self.initial_connect = false;
                    framed = try_reconnect(self.opts.clone(), &mut reactor)?;
                    break;
                } else {
                    framed = match try_reconnect(self.opts.clone(), &mut reactor) {
                        Ok(f) => f,
                        Err(_) => continue,
                    };
                    break;
                }
            }
            // Since Framed implements Split trait, we can call split on it.
            // split() gives seperate Sink and Stream objects respectively.
            let (sender, receiver) = framed.split();

            let (mut sender_tx, sender_rx) = mpsc::channel::<NetworkRequest>(1);

            let rx_future = receiver.for_each(|msg| {
                    match msg {
                        Packet::Pingresp => {
                            (*self.state.borrow_mut()).last_pingresp = true;
                        }
                        _ => (),
                    }

                    Ok(())
                })
                .map_err(|e| Error::Io(e));

            let pingtimer = self.pingtimer();
            let timer_future = pingtimer.for_each(|_| {
                    let ref mut sender_tx = sender_tx;
                    sender_tx.send(NetworkRequest::Ping).wait().unwrap();
                    Ok(())
                })
                .map_err(|e| Error::Timer(e));

            // Sender implements Sink which allows us to
            // send messages to the underlying socket connection.
            let sender_future = sender_rx.map(|r| {
                    match r {
                        // We receive a ping response from broker
                        NetworkRequest::Ping => self.ping(),
                        NetworkRequest::Publish(m) => self.publish(m),
                        _ => panic!("Misc"),
                    }
                }).map_err(|e| Error::AwaitPingResp) //TODO: Why isn't this working without map_err??
                  .and_then(|p| p)
                  .forward(sender);


            let mqtt_future = timer_future.join3(rx_future, sender_future);
            let _ = reactor.run(mqtt_future);
            println!("@@@@@@@@@@@@@@@@@@@@");
        }
    }

    fn ping(&self) -> Result<Packet, Error> {
        if !(*self.state.borrow()).last_pingresp {
            return Err(Error::AwaitPingResp);
        }
        (*self.state.borrow_mut()).last_pingresp = false;
        Ok(packet::gen_pingreq_packet())
    }

    fn publish(&self, message: Message) -> Result<Packet, Error> {
        let topic = message.topic.clone();
        let payload = message.payload.clone();
        let retain = message.retain;
        let qos = message.qos;

        match message.qos {
            QoS::AtMostOnce => (),
            QoS::AtLeastOnce => {
                let ref mut outgoing_pub = (*self.state.borrow_mut()).outgoing_pub;
                outgoing_pub.push_back(message);

                if outgoing_pub.len() > self.opts.pub_q_len as usize * 50 {
                    warn!(":( :( Outgoing Publish Queue Length growing bad --> {:?}", outgoing_pub.len());
                }
            }
            QoS::ExactlyOnce => {
                let ref mut outgoing_rec = (*self.state.borrow_mut()).outgoing_rec;
                outgoing_rec.push_back(message);

                if outgoing_rec.len() > self.opts.pub_q_len as usize * 50 {
                    warn!(":( :( Outgoing Record Queue Length growing bad --> {:?}", outgoing_rec.len());
                }
            }
        }

        Ok(packet::gen_publish_packet(topic.path, qos, None, retain, false, payload.clone()))   
    }
}

#[cfg(test)]
mod tests {
    use ::clientoptions::MqttOptions;
    use ::connection::Connection;
    use std::process::Command;
    use tokio_core::reactor::Core;
    use super::try_reconnect;
    use std::error::Error;

    #[test]
    #[should_panic]
    fn test_fail_no_listening_broker() {
        // Check no broker is listening on default port 1883
        let opts = MqttOptions::new();
        let mut reactor = Core::new().unwrap();
        let mut connection = try_reconnect(opts, &mut reactor);
        let e = connection.map_err(|e| {
            println!("{:?}", e);
            assert!(false);
        });
    }
}
