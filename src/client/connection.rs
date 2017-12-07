use std::cell::RefCell;
use std::rc::Rc;
use std::net::SocketAddr;

use futures::{Future, Sink};
use futures::stream::{Stream, SplitStream};
use futures::sync::mpsc::{Sender, Receiver};
use tokio_core::reactor::Core;
use tokio_core::net::TcpStream;
use tokio_timer::Timer;
use tokio_io::AsyncRead;
use tokio_io::codec::Framed;

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
}