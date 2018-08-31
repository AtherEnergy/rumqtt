use futures::sync::mpsc;
use futures::{Future, Sink, Stream};
use futures::stream::{SplitSink, SplitStream};
use std::net::SocketAddr;
use std::thread;
use tokio::net::TcpStream;
use tokio::timer::Deadline;
use codec::MqttCodec;
use futures::{future, stream};
use mqtt3::{Packet, PacketIdentifier};
use mqttoptions::MqttOptions;
use tokio_codec::{Decoder, Framed};
use tokio::runtime::current_thread;
use std::time::Instant;
use std::time::Duration;
use std::rc::Rc;
use std::cell::RefCell;
use std::net::ToSocketAddrs;
use client::mqttstate::MqttState;
use error::{ConnectError, NetworkReceiveError, NetworkSendError, MqttError};
use crossbeam_channel;
use client::Notification;
use client::Reply;
use client::UserRequest;


/// Composes a future which makes a new tcp connection to the broker.
/// Note that this doesn't actual connect to the broker
fn tcp_connect_future(address: &SocketAddr) -> impl Future<Item = Framed<TcpStream, MqttCodec>, Error = ConnectError> {
    TcpStream::connect(address)
        .map_err(ConnectError::from)
        .map(|stream| MqttCodec.framed(stream))
}

/// Composes a future which sends mqtt connect packet to the broker.
/// Note that this doesn't actually send the connect packet.
fn handshake_future(framed: Framed<TcpStream, MqttCodec>, 
                    mqttoptions: MqttOptions) -> (MqttState, impl Future<Item = Framed<TcpStream, MqttCodec>, Error = ConnectError>) {
    let mut mqtt_state = MqttState::new(mqttoptions);
    
    let connect_packet = future::result(mqtt_state.handle_outgoing_connect());
    let connect_future = connect_packet.and_then(|packet| {
        framed.send(Packet::Connect(packet)).map_err(|e| ConnectError::from(e))
    });
    
    (mqtt_state, connect_future)
}


/// Checks if incoming packet is mqtt connack packet and handles mqtt state
fn validate_and_handle_connack(mqtt_state: &mut MqttState, packet: Option<Packet>) -> Result<(), ConnectError> {
    match packet {
        Some(Packet::Connack(connack)) => mqtt_state.handle_incoming_connack(connack)?,
        Some(packet) => return Err(ConnectError::NotConnackPacket(packet)),
        None => return Err(ConnectError::NoResponse)
    };

    Ok(())
}

/// Composes a new future which is a combination of tcp connect + mqtt handshake
fn mqtt_connect(mqttopts: MqttOptions) -> impl Future<Item = (Framed<TcpStream, MqttCodec>, MqttState), Error = ConnectError> {
    //TODO: Remove unwraps
    let addr = &mqttopts.broker_addr.to_socket_addrs().unwrap().next().unwrap();

    let mqtt_connect = tcp_connect_future(addr).and_then(|framed| {
        let (mut mqtt_state, handshake_future) = handshake_future(framed, mqttopts);
        handshake_future.and_then(move |framed| {
            framed
                .into_future()
                .map_err(|(err, _framed)| ConnectError::from(err))
                .and_then(|(response, framed)| {
                    match validate_and_handle_connack(&mut mqtt_state, response) {
                        Ok(_) => future::ok((framed, mqtt_state)),
                        Err(e) => future::err(e)
                    }
                })
        })
    });

    mqtt_connect
}

// TODO: Add a timeout to the whole tcp connect + mqtt connect + connack wait so that our client
// TODO: won't be indefinitely blocked
//let mqtt_connect_deadline = Deadline::new(mqtt_connect_deadline, Instant::now() + Duration::from_secs(30));
//// tokio_current_thread::block_on_all(mqtt_connect_deadline);
//let mut rt = current_thread::Runtime::new().unwrap();
//rt.block_on(mqtt_connect_deadline)


//  NOTES: Don't use `wait` in eventloop thread even if you
//         are ok with blocking code. It might cause deadlocks
// https://github.com/tokio-rs/tokio-core/issues/182


pub struct Connection {
    mqtt_state: Rc<RefCell<MqttState>>,
    userrequest_rx: mpsc::Receiver<UserRequest>,
}


impl Connection {
    /// Takes mqtt options and tries to create initial connection on current thread and handles
    /// connection events in a new thread if the initial connection is successful
    pub fn run(mqttopts: MqttOptions) -> (mpsc::Sender<UserRequest>, crossbeam_channel::Receiver<Notification>) {
        let (notification_tx, notificaiton_rx) = crossbeam_channel::bounded(10);
        let (networkreply_tx, networkreply_rx) = mpsc::channel::<Reply>(10);
        let (userrequest_tx, userrequest_rx) = mpsc::channel::<UserRequest>(10);

        let mqtt_connect_future = mqtt_connect(mqttopts);
        let mqtt_connect_deadline = Deadline::new(mqtt_connect_future,
                                                  Instant::now() + Duration::from_secs(30));

        // let mut rt = current_thread::Runtime::new().unwrap();
        let (framed, mqtt_state) = current_thread::block_on_all(mqtt_connect_deadline).unwrap();

        thread::spawn(move || {
            // let mut rt = current_thread::Runtime::new().unwrap();

            let mqtt_state = Rc::new(RefCell::new(mqtt_state));
            let mut connection = Connection{mqtt_state, userrequest_rx};
            let (network_sink, network_stream) = framed.split();


            let network_receive_future = connection.network_receiver_future(notification_tx,
                                                                            networkreply_tx,
                                                                            network_stream)
                                                    .map_err(|e| {
                                                        error!("Network receive error = {:?}", e);
                                                        MqttError::NetworkReceiveError
                                                    });

            let network_transmit_future = connection.network_transmit_future(networkreply_rx, network_sink)
                                                    .map_err(|e| {
                                                        error!("Network send error = {:?}", e);
                                                        MqttError::NetworkSendError
                                                    });

            // let mqtt_future = network_transmit_future.select(network_receive_future);
            // let _ = rt.block_on(mqtt_future);

            let _out = current_thread::block_on_all(network_receive_future);
            error!("@@@@@@@@@ Reactor Exited @@@@@@@@@");
        });

        (userrequest_tx, notificaiton_rx)
    }

    /// Crates a future which handles incoming mqtt packets and notifies user over crossbeam channel
    fn network_receiver_future(&self,
                               notification_tx: crossbeam_channel::Sender<Notification>,
                               networkreply_tx: mpsc::Sender<Reply>,
                               network_stream: SplitStream<Framed<TcpStream, MqttCodec>>) -> impl Future<Item=(), Error=NetworkReceiveError> {
        let mqtt_state = self.mqtt_state.clone();

        network_stream
            .map_err(|e| {
                error!("Network receiver error = {:?}", e);
                NetworkReceiveError::from(e)
            })
            .for_each(move |packet| {
                debug!("Incoming packet = {:?}", packet);
                let notification_reply_future = future::result(mqtt_state.borrow_mut().handle_incoming_mqtt_packet(packet));
                // TODO: Can we prevent this clone?
                // cloning crossbeam channel sender everytime is a problem accordig to docs
                let networkreply_tx = networkreply_tx.clone();
                let notification_tx = notification_tx.clone();

                notification_reply_future.and_then(move |(notification, reply)| {
                    if !notification_tx.is_full() {
                        notification_tx.send(notification);
                    }
                    networkreply_tx.send(reply)
                                   .map(|_| ())
                                   .map_err(|e| NetworkReceiveError::MpscSend(e))
                }).or_else(|e| {
                    future::err(e)
                })
            })
    }

    /// Creates a future which handles all the network send requests
    fn network_transmit_future<'a>(&'a mut self,
                                   networkreply_rx: mpsc::Receiver<Reply>,
                                   network_sink: SplitSink<Framed<TcpStream, MqttCodec>>) -> impl Future<Item=(), Error=NetworkSendError> + 'a {
        let mqtt_state = self.mqtt_state.clone();
        let userrequest_rx = self.userrequest_rx.by_ref()
                                                .map(|userrequest| userrequest.into())
                                                .map_err(|e| {
                                                    error!("User request error = {:?}", e);
                                                    NetworkSendError::Blah
                                                });
        let networkreply_rx = networkreply_rx.map(|reply| reply.into())
                                             .map_err(|e| {
                                                 error!("Network reply error = {:?}", e);
                                                 NetworkSendError::Blah
                                             });;

        let last_session_publishes = mqtt_state.borrow_mut().handle_reconnection();
        let last_session_publishes = stream::iter_ok::<_, ()>(last_session_publishes)
                                            .map_err(|e| {
                                                error!("Last session publish stream error = {:?}", e);
                                                NetworkSendError::Blah
                                            });

        // NOTE: AndThen is a stream and ForEach is a future
        userrequest_rx
            .chain(last_session_publishes)
            .select(networkreply_rx)
            .and_then(move |packet: Packet| {
                match mqtt_state.borrow_mut().handle_outgoing_mqtt_packet(packet) {
                    Ok(packet) => {
                        debug!("Sending packet. {}", packet_info(&packet));
                        future::ok(packet)
                    }
                    Err(e) => future::err(e)
                }
            })
            .forward(network_sink)
            .map(|_v| ())
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

impl From<UserRequest> for Packet {
    fn from(item: UserRequest) -> Self {
        match item {
            UserRequest::MqttPublish(publish) => Packet::Publish(publish),
        }
    }
}

impl From<Reply> for Packet {
    fn from(item: Reply) -> Self {
        match item {
            Reply::PubAck(pkid) => Packet::Puback(pkid),
            _ => unimplemented!()
        }
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
        let connection = Connection::run(MqttOptions::default());
        thread::sleep(Duration::from_secs(10));
    }
}
