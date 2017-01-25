use std::net::{SocketAddr};
use std::time::{Duration, Instant};
use std::collections::VecDeque;
use std::sync::Arc;

use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;
use tokio_core::io::{Io, Framed};
use tokio_timer::{Timer};
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
    pub addr: SocketAddr,
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

    pub reactor: Core,
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
fn _try_reconnect(addr: SocketAddr, reactor: &mut Core) -> Result<Framed<TcpStream, MqttCodec>, Error> {

    let connect = generate_connect_packet("".to_string(), true, None, None);

    let f_response = TcpStream::connect(&addr, &reactor.handle()).and_then(|connection| {
        let framed = connection.framed(MqttCodec);
        let f1 = framed.send(connect);

        f1.and_then(|framed| {
                framed.into_future()
                    .and_then(|(res, stream)| Ok((res, stream)))
                    .map_err(|(err, _stream)| err)
            })
            .boxed()
    });

    let response = reactor.run(f_response);
    let (packet, frame) = response?;
    println!("{:?}", packet);
    Ok(frame)
}

impl Connection {
    pub fn start(addr: SocketAddr, 
                 opts: MqttOptions, 
                 publish_callback: Option<Arc<PublishSendableFn>>,
                 message_callback: Option<Arc<MessageSendableFn>>) -> Result<(Self, Framed<TcpStream, MqttCodec>), Error> {
        let mut reactor = Core::new().unwrap();

        let framed = _try_reconnect(addr, &mut reactor)?;

        let connection = Connection {
            addr: addr,
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

            reactor: reactor,
        };

        Ok((connection, framed))
    }

    pub fn run(&mut self, framed: Framed<TcpStream, MqttCodec>) -> Result<(), Error> {
        let (mut sender, receiver) = framed.split();
        let rx_future = receiver.for_each(|msg| {
            print!("{:?}", msg);
            Ok(())
        }).map_err(|e| Error::Io(e));

        let (mut sender_tx, sender_rx) = mpsc::channel::<NetworkRequest>(1);

        // Ping timer
        let timer = Timer::default();

        let keep_alive = if self.opts.keep_alive.is_some() {
            self.opts.keep_alive.unwrap() as u64
        } else {
            1000
        };

        let interval = timer.interval(Duration::new(keep_alive, 0));
        let timer_future = interval.for_each(|_| {
                let ref mut sender_tx = sender_tx;
                sender_tx.send(NetworkRequest::Ping).wait().unwrap();
                Ok(())
            }).map_err(|e| Error::Timer(e));

        // Sender which does network writes
        let sender_future = sender_rx.for_each(move |r| {
            let packet = match r {
                NetworkRequest::Ping => {
                    println!("{:?}", r);
                    generate_pingreq_packet()
                }
                _ => panic!("Misc")
            };

            let _ = (&mut sender).send(packet).wait();
            Ok(())
        }).map_err(|_|Error::Sender);
        
        let mqtt_future =  timer_future.join3(rx_future, sender_future);

        let _ = self.reactor.run(mqtt_future)?;
        Ok(())
    }
}
