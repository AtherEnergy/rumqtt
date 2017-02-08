use std::time::{Duration, Instant};
use std::collections::VecDeque;
use std::sync::Arc;
use std::net::{SocketAddr, ToSocketAddrs};

use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;
use tokio_core::io::{Io, Framed};
use tokio_timer::{Timer, Interval};
use futures::Future;
use futures::Sink;
use futures::Stream;
use futures::sync::mpsc;

use mqtt3::{Message, QoS, TopicPath, PacketIdentifier, SubscribeTopic};

use error::*;
use packet::*;
use codec::MqttCodec;
use clientoptions::MqttOptions;

pub type MessageSendableFn = Box<Fn(Message) + Send + Sync>;
pub type PublishSendableFn = Box<Fn(Message) + Send + Sync>;

pub struct Connection {
    pub state: MqttState,
    pub opts: MqttOptions,
    pub initial_connect: bool,
    pub await_pingresp: bool,
    pub last_flush: Instant,

    pub no_of_reconnections: u32,

    /// On message callback
    pub message_callback: Option<Arc<MessageSendableFn>>,
    /// On publish callback
    pub publish_callback: Option<Arc<PublishSendableFn>>,

    // Queues. Note: 'record' is qos2 term for 'publish'
    /// For QoS 1. Stores outgoing publishes
    pub outgoing_pub: VecDeque<Box<Message>>,
    /// For QoS 2. Store for incoming publishes to record.
    pub incoming_rec: VecDeque<Box<Message>>, //
    /// For QoS 2. Store for outgoing publishes.
    pub outgoing_rec: VecDeque<PacketIdentifier>,
    /// For Qos2. Store for outgoing `pubrel` packets.
    pub outgoing_rel: VecDeque<PacketIdentifier>,
    /// For Qos2. Store for outgoing `pubcomp` packets.
    pub outgoing_comp: VecDeque<PacketIdentifier>,

    // clean_session=false will remember subscriptions only till lives.
    // If broker crashes, all its state will be lost (most brokers).
    // client wouldn't want to loose messages after it comes back up again
    pub subscriptions: VecDeque<Vec<SubscribeTopic>>,

    // pub reactor: Core,
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

fn lookup_ipv4<A: ToSocketAddrs>(addr: A) -> SocketAddr {
    let addrs = addr.to_socket_addrs().expect("Conversion Failed");
    for addr in addrs {
        if let SocketAddr::V4(_) = addr {
            return addr;
        }
    }
    unreachable!("Cannot lookup address");
}

// DESIGN: Initial connect status should be immediately known.
//        Intermediate disconnections should be automatically reconnected
fn _try_reconnect(opts: MqttOptions,
                  reactor: &mut Core)
                  -> Result<Framed<TcpStream, MqttCodec>, Error> {

    let connect_packet = generate_connect_packet(opts.client_id, opts.clean_session, opts.keep_alive, None, None);
    let addr = lookup_ipv4(opts.addr.as_str());

    let f_response = TcpStream::connect(&addr, &reactor.handle()).and_then(|connection| {
        
        // `connection` is now a ((TcpStreamNew)) reference to a Sink + Stream iface
        // So we call framed to convert to a Framed
        let framed = connection.framed(MqttCodec);
        // Since Framed implements Sink we can call send on it.
        let f1 = framed.send(connect_packet);

        f1.and_then(|framed| {
            framed.into_future()
            .and_then(|(res, stream)| Ok((res, stream)))
            .map_err(|(err, _stream)| err)
        }).boxed()
    });

    let response = reactor.run(f_response);
    let (packet, frame) = response?;
    println!("{:?}", packet);
    Ok(frame)
}

impl Connection {
    pub fn start(opts: MqttOptions,
                 publish_callback: Option<Arc<PublishSendableFn>>,
                 message_callback: Option<Arc<MessageSendableFn>>)
                 -> Result<Self, Error> {

        let connection = Connection {
            state: MqttState::Disconnected,
            opts: opts,
            initial_connect: true,
            await_pingresp: false,
            last_flush: Instant::now(),

            no_of_reconnections: 0,

            publish_callback: publish_callback,
            message_callback: message_callback,

            // Queues
            incoming_rec: VecDeque::new(),
            outgoing_pub: VecDeque::new(),
            outgoing_rec: VecDeque::new(),
            outgoing_rel: VecDeque::new(),
            outgoing_comp: VecDeque::new(),

            // Subscriptions
            subscriptions: VecDeque::new(),
        };

        Ok(connection)
    }

    // Interval is a Stream and can be iterated over
    fn pingtimer(&self) -> Interval {
        let timer = Timer::default();

        let keep_alive = if self.opts.keep_alive > 0 {
            self.opts.keep_alive as u64
        } else {
            1000
        };

        timer.interval(Duration::new(keep_alive, 0))
    }

    pub fn run(&mut self) -> Result<(), Error> {
        let mut reactor = Core::new()?;

        'reconnect: loop {
            let framed;
            loop {
                if self.initial_connect {
                    self.initial_connect = false;
                    framed = _try_reconnect(self.opts.clone(), &mut reactor)?;
                    break;
                } else {
                    framed = match _try_reconnect(self.opts.clone(), &mut reactor) {
                        Ok(f) => f,
                        Err(_) => continue,
                    };
                    break;
                }
            }
            // Since Framed implements Split trait, we can call split on it.
            // split() gives seperate Sink and Stream objects respectively.
            let (sender, receiver) = framed.split();
            // receiver is a `Stream` type
            // for_each will process each msg on the stream we get
            let (mut sender_tx, sender_rx) = mpsc::channel::<NetworkRequest>(1);
            let rx_future = receiver.for_each(|msg| {
                    println!("Received {:?}", msg);
                    //self.await_pingresp = false;
                    Ok(())
                })
                .map_err(|e| Error::Io(e));



            // create a Stream of `Interval`

            let pingtimer = self.pingtimer();
            let timer_future = pingtimer.for_each(|_| {
                    let ref mut sender_tx = sender_tx;
                    sender_tx.send(NetworkRequest::Ping).wait().unwrap();
                    Ok(())
                })
                .map_err(|e| Error::Timer(e));

            // Sender which does network writes
            // Sender implements Sink which allows the task to 
            // send messages
            let sender_future = sender_rx.map(|r| {
                    match r {
                        // We receive a ping response from broker
                        NetworkRequest::Ping => {
                            if self.await_pingresp {
                                return Err(Error::AwaitPingResp);
                            }
                            self.await_pingresp = true;
                            Ok(generate_pingreq_packet())
                        }
                        _ => panic!("Misc"),
                    }
                }).map_err(|e| Error::Sender)
                  .and_then(|p| p)
                  .forward(sender);

            //let sender_future.

            let mqtt_future = timer_future.join3(rx_future, sender_future);

            let e = reactor.run(mqtt_future);
            println!("@@@@@@@@@@@@@@@@@@@@");
        }
    }
}
