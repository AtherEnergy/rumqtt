use futures::sync::mpsc::{self, Sender, Receiver};
use futures::{Future, Sink, Stream};
use futures::stream::{SplitSink, SplitStream};
use std::net::SocketAddr;
use std::thread;
use tokio::net::TcpStream;
use tokio::timer::Deadline;
use codec::MqttCodec;
use error::ConnectError;
use futures::future;
use mqtt3::Packet;
use mqttoptions::MqttOptions;
use tokio_codec::{Decoder, Framed};
use tokio_io::AsyncRead;
use tokio::runtime::current_thread;
use std::time::Instant;
use std::time::Duration;
use std::rc::Rc;
use std::cell::RefCell;
use client::mqttstate::MqttState;
use error::NetworkReceiveError;
use tokio::timer::DeadlineError;
use crossbeam_channel;
use client::Notification;
use mqtt3::PacketIdentifier;


/// Composes a future which makes a new tcp connection to the broker.
/// Note this doesn't actual connect to the broker
fn tcp_connect_future(address: &SocketAddr) -> impl Future<Item = Framed<TcpStream, MqttCodec>, Error = ConnectError> {
    TcpStream::connect(address)
        .map_err(ConnectError::from)
        .map(|stream| MqttCodec.framed(stream))
}

/// Composes a future which sends mqtt connect packet to the broker.
/// Note that this doesn't actually send the connect packet.
fn handshake_future(framed: Framed<TcpStream, MqttCodec>) -> impl Future<Item = Framed<TcpStream, MqttCodec>, Error = ConnectError> {
    let mqttoptions = MqttOptions::default();
    let connect_packet = mqttoptions.connect_packet();

    let framed = framed.send(Packet::Connect(connect_packet));
    framed.map_err(|err| ConnectError::from(err))
}

/// Composes a new future which is a combination of tcp connect + handshake + connack receive.
/// This function also runs to eventloop to create mqtt connection and returns `Framed`
fn mqtt_connect(address: &SocketAddr, mqttopts: MqttOptions) -> Result<Connection, DeadlineError<ConnectError>> {
    let mqtt_connect_deadline = tcp_connect_future(address).and_then(|framed| {
        handshake_future(framed).and_then(|framed| {
            framed
                .into_future()
                .map_err(|(err, _framed)| ConnectError::from(err))
                .and_then(|(response, framed)| {
                    let mut mqtt_state = MqttState::new(mqttopts);

                    if let Some(Packet::Connack(connack)) = response {
                        match mqtt_state.handle_incoming_connack(connack) {
                            Ok(v) => v,
                            Err(e) => return future::err(e),
                        }
                    } else {
                        panic!("Expected connack packet. Got = {:?}", response);
                    }

                    let connection = Connection {
                        mqtt_state: Rc::new(RefCell::new(mqtt_state)),
                        framed: Some(framed)
                    };

                    future::ok(connection)
                })
        })
    });

    // TODO: Add a timeout to the whole tcp connect + mqtt connect + connack wait so that our client
    // TODO: won't be indefinitely blocked
    let mqtt_connect_deadline = Deadline::new(mqtt_connect_deadline, Instant::now() + Duration::from_secs(30));
    // tokio_current_thread::block_on_all(mqtt_connect_deadline);
    let mut rt = current_thread::Runtime::new().unwrap();
    rt.block_on(mqtt_connect_deadline)
}


//  NOTES: Don't use `wait` in eventloop thread even if you
//         are ok with blocking code. It might cause deadlocks
// https://github.com/tokio-rs/tokio-core/issues/182


struct Connection {
    mqtt_state: Rc<RefCell<MqttState>>,
    framed: Option<Framed<TcpStream, MqttCodec>>
}

impl Connection {
    pub fn run(&mut self) -> crossbeam_channel::Receiver<Notification> {
        let framed = self.framed.take().unwrap();
        let (network_sink, network_stream) = framed.split();

        let (notification_tx, notificaiton_rx) = crossbeam_channel::bounded(10);


        notificaiton_rx
    }

    fn network_receiver_future(&self,
                               notification_tx: crossbeam_channel::Sender<Notification>,
                               network_stream: SplitStream<Framed<TcpStream, MqttCodec>>) -> impl Future<Item=(), Error=NetworkReceiveError> {
        let mqtt_state = self.mqtt_state.clone();

        network_stream
            .map_err(|e| NetworkReceiveError::from(e))
            .for_each(move |packet| {
                let (notification, reply) = match mqtt_state.borrow_mut().handle_incoming_mqtt_packet(packet) {
                    Ok(v) => v,
                    Err(e) => return future::err(e),
                };

                if !notification_tx.is_full() {
                    notification_tx.send(notification)
                }

                future::ok(())
        })
    }
}



fn packet_info(packet: &Packet) -> String {
    match packet {
        Packet::Publish(p) => format!(
            "topic = {}, \
             qos = {:?}, \
             pkid = {:?}, \
             payload size = {:?} bytes", p.topic_name, p.qos, p.pid, p.payload.len()),

        _ => format!("{:?}", packet)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use pretty_env_logger;

    #[test]
    fn it_works() {
        pretty_env_logger::init();
        let connection = mqtt_connect(&"127.0.0.1:1883".parse().unwrap(), MqttOptions::default()).unwrap();
        thread::sleep_ms(10000);
    }
}
