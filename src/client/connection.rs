use client::mqttstate::MqttState;
use client::mqttasync;
use client::network::stream::NetworkStream;
use client::Notification;
use client::Request;
use codec::MqttCodec;
use crossbeam_channel;
use error::{ConnectError, NetworkError, PollError};
use futures::stream::SplitStream;
use futures::sync::mpsc;
use futures::{future, stream};
use futures::{Future, Sink, Stream};
use mqtt3::Packet;
use mqttoptions::{ConnectionMethod, MqttOptions, ReconnectOptions};
use std::cell::RefCell;
use std::rc::Rc;
use std::thread;
use std::time::Duration;
use tokio::runtime::current_thread;
use tokio_codec::Framed;
use tokio_timer::Timeout;
//  NOTES: Don't use `wait` in eventloop thread even if you
//         are ok with blocking code. It might cause deadlocks
//  https://github.com/tokio-rs/tokio-core/issues/182

type MqttFramed = Framed<NetworkStream, MqttCodec>;

pub struct Connection {
    mqtt_state: Rc<RefCell<MqttState>>,
    notification_tx: crossbeam_channel::Sender<Notification>,
    mqttoptions: MqttOptions,
}

impl Connection {
    /// Takes mqtt options and tries to create initial connection on current thread and handles
    /// connection events in a new thread if the initial connection is successful
    pub fn run(
        mqttoptions: MqttOptions)
        -> Result<(mpsc::Sender<Request>, crossbeam_channel::Receiver<Notification>), ConnectError>
    {
        let (notification_tx, notificaiton_rx) = crossbeam_channel::bounded(10);
        let (userrequest_tx, userrequest_rx) = mpsc::channel::<Request>(10);
        let (connection_tx, connection_rx) = crossbeam_channel::bounded(1);
        let reconnect_option = mqttoptions.reconnect;

        thread::spawn(move || {
            let mqtt_state = Rc::new(RefCell::new(MqttState::new(mqttoptions.clone())));
            let mut connection = Connection { mqtt_state,
                                              notification_tx,
                                              mqttoptions };

            connection.mqtt_eventloop(connection_tx, userrequest_rx)
        });

        match reconnect_option {
            ReconnectOptions::AfterFirstSuccess(_) | ReconnectOptions::Never => {
                connection_rx.recv()??;
                Ok((userrequest_tx, notificaiton_rx))
            }
            _ => Ok((userrequest_tx, notificaiton_rx)),
        }
    }

    fn mqtt_eventloop(&mut self,
                      connection_tx: crossbeam_channel::Sender<Result<(), ConnectError>>,
                      userrequest_rx: mpsc::Receiver<Request>) {
        let mut connection_count = 1;
        let reconnect_option = self.mqttoptions.reconnect;
        let mut previous_request_stream = self.user_request_stream(userrequest_rx);

        'reconnection: loop {
            let mqtt_connect_future = self.mqtt_connect();
            let mqtt_connect_deadline = Timeout::new(mqtt_connect_future, Duration::from_secs(30));

            // NOTE: We need to use same reactor across threads because io resources (framed) will
            //       bind to reactor lazily.
            //       You'll face `reactor gone` error if `framed` is used again with a new recator
            let mut rt = current_thread::Runtime::new().unwrap();
            let framed = match rt.block_on(mqtt_connect_deadline) {
                Ok(framed) => {
                    if connection_count == 1 {
                        connection_tx.send(Ok(())).unwrap();
                    }
                    connection_count += 1;
                    framed
                }
                Err(e) => {
                    error!("Connection error = {:?}", e);
                    let error = match e.into_inner() {
                        Some(e) => Err(e),
                        None => Err(ConnectError::Timeout),
                    };

                    match reconnect_option {
                        ReconnectOptions::AfterFirstSuccess(_) if connection_count == 1 => {
                            connection_tx.send(error).unwrap();
                            break;
                        }
                        ReconnectOptions::AfterFirstSuccess(time) => {
                            thread::sleep(Duration::from_secs(time))
                        }
                        ReconnectOptions::Always(time) => thread::sleep(Duration::from_secs(time)),
                        ReconnectOptions::Never => {
                            connection_tx.send(error).unwrap();
                            break;
                        }
                    }
                    continue 'reconnection;
                }
            };

            debug!("Mqtt connection successful!!");

            let (network_sink, network_stream) = framed.split();
            let network_reply_stream = self.network_reply_stream(network_stream);
            let network_request_stream = self.network_request_stream(previous_request_stream);

            let mqtt_stream = mqttasync::new(network_reply_stream, network_sink, network_request_stream);

            let (mqtt_sink, mqtt_stream) = mqtt_stream.split();
            let mqtt_future = mqtt_stream.forward(mqtt_sink);


            match rt.block_on(mqtt_future) {
                Ok(_) => panic!("Shouldn't happen"),
                Err(PollError::Network((e, s))) => {
                    error!("Event loop disconnect. Error = {:?}", e);
                    previous_request_stream = s;
                }
                Err(PollError::UserRequest(_)) => panic!("User req error"),
                Err(PollError::StreamClosed(s)) => {
                    error!("Stream closed error");
                    previous_request_stream = s;
                }
            }

            match reconnect_option {
                ReconnectOptions::AfterFirstSuccess(time) => {
                    let time = Duration::from_secs(time);
                    thread::sleep(time);
                    continue 'reconnection
                }
                ReconnectOptions::Always(time) => {
                    let time = Duration::from_secs(time);
                    thread::sleep(time);
                    continue 'reconnection
                },
                ReconnectOptions::Never => break 'reconnection,
            }
        }
    }

    /// Resolves dns with blocking API and composes a future
    /// which makes a new tcp or tls connection to the broker.
    /// Note that this doesn't actual connect to the broker
    fn tcp_connect_future(&self) -> impl Future<Item = MqttFramed, Error = ConnectError> {
        let host = &self.mqttoptions.broker_addr;
        let port = self.mqttoptions.port;
        let connection_method = self.mqttoptions.connection_method.clone();
        let builder = NetworkStream::builder();

        let builder = match connection_method {
            ConnectionMethod::Tls(ca, Some((cert, key))) => builder.add_certificate_authority(&ca)
                                                                   .add_client_auth(&cert, &key),
            ConnectionMethod::Tls(ca, None) => builder.add_certificate_authority(&ca),
            ConnectionMethod::Tcp => builder,
        };

        builder.connect(host, port)
    }

    /// Composes a new future which is a combination of tcp connect + mqtt handshake
    fn mqtt_connect(&self) -> impl Future<Item = MqttFramed, Error = ConnectError> {
        let mqtt_state = self.mqtt_state.clone();
        let tcp_connect_future = self.tcp_connect_future();
        let connect_packet = self.mqtt_state
                                 .borrow_mut()
                                 .handle_outgoing_connect()
                                 .unwrap();

        tcp_connect_future.and_then(move |framed| {
                              let packet = Packet::Connect(connect_packet);
                              framed.send(packet).map_err(|e| ConnectError::Io(e))
                          })
                          .and_then(|framed| {
                              framed.into_future()
                                    .map_err(|(err, _framed)| ConnectError::Io(err))
                          })
                          .and_then(move |(response, framed)| {
                              debug!("Mqtt connect response = {:?}", response);
                              let mut mqtt_state = mqtt_state.borrow_mut();
                              check_and_validate_connack(response, framed, &mut mqtt_state)
                          })
    }

    /// Handles all incoming network packets (including sending notifications to user over crossbeam
    /// channel) and creates a stream of packets to send on network
    fn network_reply_stream(&self,
                            network_stream: SplitStream<MqttFramed>)
                            -> impl Stream<Item = Packet, Error = NetworkError> {
        let mqtt_state_in = self.mqtt_state.clone();
        let mqtt_state_out = self.mqtt_state.clone();
        let keep_alive = self.mqttoptions.keep_alive;
        let network_stream = Timeout::new(network_stream, keep_alive);

        // TODO: prevent this clone?
        // cloning crossbeam channel sender everytime is a problem according to docs
        let notification_tx = self.notification_tx.clone();
        network_stream.map_err(|e| NetworkError::TimeOut(e))
                      .and_then(move |packet| {
                          debug!("Incoming packet = {:?}", packet_info(&packet));
                          let reply = mqtt_state_in.borrow_mut().handle_incoming_mqtt_packet(packet);
                          future::result(reply)
                      })
                      .and_then(move |(notification, reply)| {
                          let notification_tx = notification_tx.clone();
                          handle_notification(notification, &notification_tx);
                          future::ok(reply)
                      })
                      .or_else(move |e| match e {
                          NetworkError::TimeOut(ref e) if e.is_elapsed() => {
                              let ping = Packet::Pingreq;
                              match mqtt_state_out.borrow_mut().handle_outgoing_mqtt_packet(ping) {
                                  Ok(_) => future::ok(Request::Ping),
                                  Err(e) => future::err(e)
                              }
                          }
                          _ => {
                              error!("Stream failed. Error = {:?}", e);
                              future::err(e)
                          },
                      })
                      .filter(|reply| should_forward_packet(reply))
                      .and_then(move |packet| future::ok(packet.into()))
    }

    fn network_request_stream(&mut self, previous_request_stream: impl Stream<Item = Packet, Error = NetworkError>) -> impl Stream<Item = Packet, Error = NetworkError> {
        let mqtt_state = self.mqtt_state.clone();
        let last_session_publishes = mqtt_state.borrow_mut().handle_reconnection();

        let last_session_stream = stream::iter_ok::<_, ()>(last_session_publishes).map_err(|e| {
            error!("Last session publish stream error = {:?}", e);
            NetworkError::Blah
        });

        last_session_stream.chain(previous_request_stream)
    }

    /// Handles all incoming user and session requests and creates a stream of packets to send
    /// on network
    /// All the remaining packets in the last session (when cleansession = false) will be prepended
    /// to user request stream to ensure that they are handled first. This cleanly handles last
    /// session stray (even if disconnect happens while sending last session data)because we always
    /// get back this stream from reactor after disconnection.
    fn user_request_stream(&mut self,
                           userrequest_rx: mpsc::Receiver<Request>)
                           -> impl Stream<Item = Packet, Error = NetworkError> {
        let mqtt_state = self.mqtt_state.clone();

        let userrequest_rx = userrequest_rx.map_err(|e| {
                                               error!("User request error = {:?}", e);
                                               NetworkError::Blah
                                           })
                                           .and_then(move |userrequest| {
                                               let mut mqtt_state = mqtt_state.borrow_mut();
                                               validate_userrequest(userrequest, &mut mqtt_state)
                                           });
        let mqtt_state = self.mqtt_state.clone();
        userrequest_rx
            .and_then(move |packet: Packet| {
                future::result(mqtt_state.borrow_mut().handle_outgoing_mqtt_packet(packet))
            })
    }
}

fn validate_userrequest(userrequest: Request,
                        mqtt_state: &mut MqttState)
                        -> impl Future<Item = Packet, Error = NetworkError> {
    match userrequest {
        Request::Reconnect(mqttoptions) => {
            mqtt_state.opts = mqttoptions;
            future::err(NetworkError::UserReconnect)
        }
        _ => future::ok(userrequest.into()),
    }
}

fn handle_notification(notification: Notification,
                       notification_tx: &crossbeam_channel::Sender<Notification>) {
    match notification {
        Notification::None => (),
        _ => match notification_tx.try_send(notification) {
            Ok(()) => (),
            Err(e) => error!("Notification send failed. Error = {:?}", e),
        },
    }
}

/// Checks if incoming packet is mqtt connack packet. Useful after mqtt
/// connect when we are waiting for connack but not any other packet.
fn check_and_validate_connack(packet: Option<Packet>,
                              framed: MqttFramed,
                              mqtt_state: &mut MqttState)
                              -> impl Future<Item = MqttFramed, Error = ConnectError> {
    match packet {
        Some(Packet::Connack(connack)) => match mqtt_state.handle_incoming_connack(connack) {
            Err(err) => future::err(err),
            _ => future::ok(framed),
        },
        Some(packet) => future::err(ConnectError::NotConnackPacket(packet)),
        None => future::err(ConnectError::NoResponse),
    }
}

fn should_forward_packet(reply: &Request) -> bool {
    match reply {
        Request::None => false,
        _ => true,
    }
}

fn packet_info(packet: &Packet) -> String {
    match packet {
        Packet::Publish(p) => format!("topic = {}, \
                                       qos = {:?}, \
                                       pkid = {:?}, \
                                       payload size = {:?} bytes",
                                      p.topic_name,
                                      p.qos,
                                      p.pid,
                                      p.payload.len()),

        _ => format!("{:?}", packet),
    }
}

impl From<Request> for Packet {
    fn from(item: Request) -> Self {
        match item {
            Request::Publish(publish) => Packet::Publish(publish),
            Request::PubAck(pkid) => Packet::Puback(pkid),
            Request::PubRec(pkid) => Packet::Pubrec(pkid),
            Request::PubRel(pkid) => Packet::Pubrel(pkid),
            Request::PubComp(pkid) => Packet::Pubcomp(pkid),
            Request::Ping => Packet::Pingreq,
            Request::Disconnect => Packet::Disconnect,
            Request::Subscribe(subscribe) => Packet::Subscribe(subscribe),
            _ => unimplemented!(),
        }
    }
}
