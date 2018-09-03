use mqtt3::Connack;
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
use tokio::timer::Timeout;
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


//  NOTES: Don't use `wait` in eventloop thread even if you
//         are ok with blocking code. It might cause deadlocks
//  https://github.com/tokio-rs/tokio-core/issues/182


pub struct Connection {
    mqtt_state: Rc<RefCell<MqttState>>,
    userrequest_rx: mpsc::Receiver<UserRequest>,
    networkreply_tx: mpsc::Sender<Reply>,
    networkreply_rx: mpsc::Receiver<Reply>,
    notification_tx: crossbeam_channel::Sender<Notification>,
    mqttoptions: MqttOptions
}

impl Connection {
    /// Takes mqtt options and tries to create initial connection on current thread and handles
    /// connection events in a new thread if the initial connection is successful
    pub fn run(mqttoptions: MqttOptions) -> (mpsc::Sender<UserRequest>, crossbeam_channel::Receiver<Notification>) {
        let (notification_tx, notificaiton_rx) = crossbeam_channel::bounded(10);
        let (networkreply_tx, networkreply_rx) = mpsc::channel::<Reply>(10);
        let (userrequest_tx, userrequest_rx) = mpsc::channel::<UserRequest>(10);

        thread::spawn(move || {
            
            let mqtt_state = Rc::new(RefCell::new(MqttState::new(mqttoptions.clone())));
            let mut connection = Connection {
                mqtt_state, userrequest_rx, 
                networkreply_tx, 
                networkreply_rx, 
                notification_tx,
                mqttoptions
            };

            'reconnection: loop {
                let mqtt_connect_future = connection.mqtt_connect();
                let mqtt_connect_deadline = Timeout::new(mqtt_connect_future,
                                                      Instant::now() + Duration::from_secs(30));

                // NOTE: We need to use same reactor across threads because io resources (framed) will
                //       bind to reactor lazily.
                //       You'll face `reactor gone` error if `framed` is used again with a new recator
                let mut rt = current_thread::Runtime::new().unwrap();
                let framed = match rt.block_on(mqtt_connect_deadline) {
                    Ok(framed) => framed,
                    Err(e) => {
                        error!("Connection error = {:?}", e);
                        thread::sleep_ms(2000);
                        continue 'reconnection
                    }
                };
                
                debug!("Mqtt connection successful!!");

                let mqtt_future = connection.mqtt_future(framed);
                
                if let Err(e) = rt.block_on(mqtt_future) {
                    error!("Mqtt eventloop error = {:?}", e);
                    thread::sleep_ms(2000);
                    continue 'reconnection
                }

                error!("@@@@@@@@@ Reactor Exited @@@@@@@@@");
            }
        });

        (userrequest_tx, notificaiton_rx)
    }

    /// Resolves dns with blocking API and composes a future
    /// which makes a new tcp or tls connection to the broker.
    /// Note that this doesn't actual connect to the broker
    fn tcp_connect_future(&self) -> impl Future<Item = Framed<TcpStream, MqttCodec>, Error = ConnectError> {
        // TODO: Remove unwraps
        // TODO: This call will also do dns resolution. Find a way to add timeout to this call
        let address = self.mqttoptions.broker_addr.to_socket_addrs().unwrap().next().unwrap();

        TcpStream::connect(&address)
            .map_err(ConnectError::from)
            .map(|stream| MqttCodec.framed(stream))
    }

    /// Composes a new future which is a combination of tcp connect + mqtt handshake
    fn mqtt_connect(&self) -> impl Future<Item = Framed<TcpStream, MqttCodec>, Error = ConnectError> {
        let mqtt_state = self.mqtt_state.clone();
        let tcp_connect_future = self.tcp_connect_future();
        let connect_packet = future::result(self.mqtt_state.borrow_mut().handle_outgoing_connect());

        tcp_connect_future.and_then(move |framed| connect_packet.and_then(move |packet| { // send mqtt connect packet
            framed.send(Packet::Connect(packet)).map_err(|e| ConnectError::from(e))
        })
        .and_then(move |framed| {
            framed
                .into_future()
                .map_err(|(err, _framed)| ConnectError::from(err))
                .and_then(move |(response, framed)| { // receive mqtt connack packet
                    debug!("Mqtt connect response = {:?}", response);
                    let mut mqtt_state = mqtt_state.borrow_mut();
                    match check_if_connack(response) {
                        Ok(connack) => {
                            if let Err(err) = mqtt_state.handle_incoming_connack(connack) {
                                return future::err(err)
                            }
                            future::ok(framed)
                        }
                        Err(e) => future::err(e)
                    }
                })
        }))
    }

    fn mqtt_future<'a>(&'a mut self,
                       framed: Framed<TcpStream, MqttCodec>) -> impl Future<Item = (), Error = MqttError> + 'a {
        
        let (network_sink, network_stream) = framed.split();

        let network_receive_future = self.network_receiver_future(network_stream)
                                         .map_err(|e| {
                                             error!("Network receive error = {:?}", e);
                                             MqttError::NetworkReceiveError
                                         });
        
        let network_transmit_future = self.network_transmit_future(network_sink)
                                          .map_err(|e| {
                                              error!("Network send error = {:?}", e);
                                              MqttError::NetworkSendError
                                          });
        
        let mqtt_future = network_transmit_future.select(network_receive_future); 
        mqtt_future.map(|_| ()).map_err(|(err, _select)| err)
    }

    /// Crates a future which handles incoming mqtt packets and notifies user over crossbeam channel
    fn network_receiver_future(&self,
                               network_stream: SplitStream<Framed<TcpStream, MqttCodec>>) -> impl Future<Item=(), Error=NetworkReceiveError> {
        let mqtt_state = self.mqtt_state.clone();
        
        // TODO: Can we prevent this clone?
        // cloning crossbeam channel sender everytime is a problem accordig to docs
        let networkreply_tx = self.networkreply_tx.clone();
        let notification_tx = self.notification_tx.clone();
        network_stream
            .map_err(|e| {
                error!("Network receiver error = {:?}", e);
                NetworkReceiveError::from(e)
            })
            .for_each(move |packet| {
                debug!("Incoming packet = {:?}", packet);
                let notification_reply_future = future::result(mqtt_state.borrow_mut().handle_incoming_mqtt_packet(packet));
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
                                   network_sink: SplitSink<Framed<TcpStream, MqttCodec>>) -> impl Future<Item=(), Error=NetworkSendError> + 'a {
        let mqtt_state = self.mqtt_state.clone();
        let userrequest_rx = self.userrequest_rx.by_ref()
                                                .map(|userrequest| userrequest.into())
                                                .map_err(|e| {
                                                    error!("User request error = {:?}", e);
                                                    NetworkSendError::Blah
                                                });
        let networkreply_rx = self.networkreply_rx.by_ref()
                                                  .map(|reply| reply.into())
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

/// Checks if incoming packet is mqtt connack packet. Useful after mqtt
/// connect when we are waiting for connack but not any other packet.
pub fn check_if_connack(packet: Option<Packet>) -> Result<Connack, ConnectError> {        
    match packet {
        Some(Packet::Connack(connack)) => Ok(connack),
        Some(packet) => return Err(ConnectError::NotConnackPacket(packet)),
        None => return Err(ConnectError::NoResponse)
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
