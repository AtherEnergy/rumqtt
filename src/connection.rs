use std::net::{SocketAddr};
use std::time::{Duration, Instant};

use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;
use tokio_core::io::{Io, Framed};
use tokio_timer::{Timer};
use futures::Future;
use futures::Sink;
use futures::Stream;
use futures::sync::mpsc;

use mqtt3::{Message, QoS, TopicPath};

use error::*;
use packet::*;
use codec::MqttCodec;
use clientoptions::MqttOptions;

pub struct Connection {
    pub addr: SocketAddr,
    pub state: MqttState,
    pub opts: MqttOptions,
    pub initial_connect: bool,
    pub await_pingresp: bool,
    pub last_flush: Instant,

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
    pub fn start(addr: SocketAddr, opts: MqttOptions) -> Result<(Self, Framed<TcpStream, MqttCodec>), Error> {
        let mut reactor = Core::new().unwrap();

        let framed = _try_reconnect(addr, &mut reactor)?;

        let connection = Connection {
            addr: addr,
            state: MqttState::Disconnected,
            opts: opts,
            initial_connect: true,
            await_pingresp: false,
            last_flush: Instant::now(),

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
        let interval = timer.interval(Duration::new(5, 0));
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
