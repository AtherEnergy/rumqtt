use futures::sync::mpsc::{self, Sender, Receiver};
use futures::{Future, Sink, Stream};
use futures::stream::{SplitSink, SplitStream};
use std::net::SocketAddr;
use std::thread;
use tokio::net::TcpStream;
use tokio::timer::Deadline;
use codec::MqttCodec;
use error::ConnectError;
use futures::{future, stream};
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
use client::Reply;
use error::NetworkSendError;
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
fn handshake_future(framed: Framed<TcpStream, MqttCodec>) -> impl Future<Item = Framed<TcpStream, MqttCodec>, Error = ConnectError> {
    let mqttoptions = MqttOptions::default();
    let connect_packet = mqttoptions.connect_packet();

    let framed = framed.send(Packet::Connect(connect_packet));
    framed.map_err(|err| ConnectError::from(err))
}

/// Future which does a network call to receive connack packet.
/// TODO: Fix the panic. Disconnect the client
fn receive_connack_future(framed: Framed<TcpStream, MqttCodec>) -> impl Future<Item = Framed<TcpStream, MqttCodec>, Error = ConnectError> {
    framed
        .into_future()
        .map_err(|(err, _framed)| ConnectError::from(err))
        .and_then(|(response, framed)| {
            let mut mqtt_state = MqttState::new(mqttopts)

            if let Some(Packet::Connack(connack)) = response {
                match mqtt_state.handle_incoming_connack(connack) {
                    Ok(v) => v,
                    Err(e) => return future::err(e),
                }
            } else {
                panic!("Expected connack packet. Got = {:?}", response);
                future::ok((framed, mqtt_state))
            }
        )
}

/// Composes a new future which is a combination of tcp connect + handshake + connack receive.
/// This function also runs to eventloop to create mqtt connection and returns `Framed`
fn mqtt_connect(mqttopts: MqttOptions) -> impl Future<Item = (Framed<TcpStream, MqttCodec>, MqttState), Error = ConnectError> {
    let addr = &mqttopts.broker_addr.parse().unwrap();

    let mqtt_connect = tcp_connect_future(addr).and_then(|framed| {
        handshake_future(framed)
            .and_then(|framed| {
                receive_connack_future(framed)
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


struct Connection {
    mqtt_state: Rc<RefCell<MqttState>>,
    userrequest_rx: mpsc::Receiver<UserRequest>,
}


impl Connection {
    pub fn run(mqttopts: MqttOptions) -> crossbeam_channel::Receiver<Notification> {
        let (notification_tx, notificaiton_rx) = crossbeam_channel::bounded(10);
        let (networkreply_tx, networkreply_rx) = mpsc::channel::<Reply>(10);
        let (userrequest_tx, userrequest_rx) = mpsc::channel::<UserRequest>(10);


        let mqtt_connect_future = mqtt_connect(mqttopts);
        let mqtt_connect_deadline = Deadline::new(mqtt_connect_future,
                                                  Instant::now() + Duration::from_secs(30));

        let mut rt = current_thread::Runtime::new().unwrap();
        let (framed, mqtt_state) = rt.block_on(mqtt_connect_deadline).unwrap();

        thread::spawn(move || {
            let mut rt = current_thread::Runtime::new().unwrap();

            let mqtt_state = Rc::new(RefCell::new(mqtt_state));
            let mut connection = Connection{mqtt_state, userrequest_rx};
            let (network_sink, network_stream) = framed.split();


            let network_receive_future = connection.network_receiver_future(notification_tx,
                                                                            networkreply_tx,
                                                                            network_stream);

            let network_transmit_future = connection.network_transmit_future(network_sink);

            let out = rt.block_on(network_transmit_future);
            info!("Reactor result = {:?}", out);
        });

        notificaiton_rx
    }

    fn network_receiver_future(&self,
                               notification_tx: crossbeam_channel::Sender<Notification>,
                               networkreply_tx: mpsc::Sender<Reply>,
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

    fn network_transmit_future<'a>(&'a mut self, network_sink: SplitSink<Framed<TcpStream, MqttCodec>>)
        -> impl Future<Item=(), Error=NetworkSendError> + 'a {
        let mqtt_state = self.mqtt_state.clone();
        let request_rx = self.userrequest_rx.by_ref();

        let last_session_publishes = mqtt_state.borrow_mut().handle_reconnection();
        let mut last_session_publishes = stream::iter_ok::<_, ()>(last_session_publishes);

        // NOTE: AndThen is a stream and ForEach is a future
        request_rx
            .map_err(|e| NetworkSendError::Blah)
            .and_then(move |request| {
                let packet = match request {
                    UserRequest::MqttPublish(p) => Packet::Publish(p)
                };

                match mqtt_state.borrow_mut().handle_outgoing_mqtt_packet(packet) {
                    Ok(packet) => {
                        debug!("Sending packet. {}", packet_info(&packet));
                        future::ok(packet)
                    }
                    Err(e) => {
                        error!("Handling outgoing packet failed. Error = {:?}", e);
                        future::err(e)
                    }
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
