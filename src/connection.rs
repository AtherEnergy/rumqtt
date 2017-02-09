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

use mqtt3::{Message, QoS, TopicPath, PacketIdentifier, SubscribeTopic};

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

/// States which needs mutation. We keep state as a seperate struct to avoid compile time borrow checker errors.
pub struct State {
    /// Flag which describes the ping response ack state, and is initially true.
    last_pingresp: bool
}

impl Default for State {
    fn default() -> Self {
        State {
            last_pingresp:true
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
fn _try_reconnect(opts: MqttOptions,
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
        }).boxed()
    });

    let response = reactor.run(f_response);
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

            let (mut sender_tx, sender_rx) = mpsc::channel::<NetworkRequest>(1);

            let rx_future = receiver.for_each(|msg| {
                    (*self.state.borrow_mut()).last_pingresp = true;
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
                        NetworkRequest::Ping => {
                            if !(*self.state.borrow()).last_pingresp {
                                return Err(Error::AwaitPingResp);
                            }
                            (*self.state.borrow_mut()).last_pingresp = false;
                            Ok(packet::gen_pingreq_packet())
                        }
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
}

#[cfg(test)]
mod tests {
    use ::clientoptions::MqttOptions;
    use ::connection::Connection;
    use std::process::Command;

    #[test]
    fn test_fail_no_listening_broker() {
        // Check no broker is listening on default port 1883
        let status = Command::new("lsof")
                         .arg("-i")
                         .arg(":1883")
                         .arg("-S")
                         .status()
                         .expect("failed to execute process");
        assert!(!status.success());
        let opts = MqttOptions::new();
        let mut connection = Connection::start(opts, None, None).unwrap();
        let e = connection.run();
        assert!(e.is_err());
    }
}
