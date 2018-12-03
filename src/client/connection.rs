use client::{
    mqttasync,
    mqttstate::MqttState,
    network::stream::NetworkStream,
    prepend::{Prepend, StreamExt},
    Notification,
    Request,
};
use codec::MqttCodec;
use crossbeam_channel;
use error::{ConnectError, NetworkError, PollError};
use futures::{
    future,
    stream::{self, SplitStream},
    sync::mpsc,
    Future,
    Sink,
    Stream,
};
use mqtt311::Packet;
use mqttoptions::{ConnectionMethod, MqttOptions, ReconnectOptions};
use std::{cell::RefCell, rc::Rc, thread, time::Duration};
use tokio::runtime::current_thread;
use tokio_codec::Framed;
use tokio_timer::Timeout;
use crossbeam_channel::Sender;
use futures::sync::mpsc::Receiver;
use client::UserHandle;
use tokio_timer::timeout;
use client::Command;
use mqttoptions::Proxy;

//  NOTES: Don't use `wait` in eventloop thread even if you
//         are ok with blocking code. It might cause deadlocks
//  https://github.com/tokio-rs/tokio-core/issues/182

pub struct Connection {
    mqtt_state: Rc<RefCell<MqttState>>,
    notification_tx: Sender<Notification>,
    connection_tx: Option<Sender<Result<(), ConnectError>>>,
    connection_count: u32,
    mqttoptions: MqttOptions,
}

impl Connection {
    /// Takes mqtt options and tries to create initial connection on current thread and handles
    /// connection events in a new thread if the initial connection is successful
    pub fn run(mqttoptions: MqttOptions) -> Result<UserHandle, ConnectError> {
        let (notification_tx, notification_rx) = crossbeam_channel::bounded(10);
        let (request_tx, request_rx) = mpsc::channel::<Request>(10);
        let (command_tx, command_rx) = mpsc::channel::<Command>(5);

        let (connection_tx, connection_rx) = crossbeam_channel::bounded(1);
        let reconnect_option = mqttoptions.reconnect_opts();

        // start the network thread to handle all mqtt network io
        thread::spawn(move || {
            let mqtt_state = Rc::new(RefCell::new(MqttState::new(mqttoptions.clone())));
            let mut connection = Connection { mqtt_state,
                                              notification_tx,
                                              connection_tx: Some(connection_tx),
                                              connection_count: 0,
                                              mqttoptions };

            connection.mqtt_eventloop(request_rx, command_rx)
        });


        // return user handle to client to send requests and handle notifications
        let user_handle = UserHandle{request_tx, command_tx, notification_rx};

        match reconnect_option {
            ReconnectOptions::AfterFirstSuccess(_) => {
                connection_rx.recv()??;
                Ok(user_handle)
            }
            ReconnectOptions::Never => {
                connection_rx.recv()??;
                Ok(user_handle)
            }
            ReconnectOptions::Always(_) => Ok(user_handle)
        }
    }

    // NOTE: We need to use same reactor across threads because io resources (framed) will
    //       bind to reactor lazily.
    //       You'll face `reactor gone` error if `framed` is used again with a new recator
    fn mqtt_eventloop(&mut self, request_rx: Receiver<Request>, command_rx: Receiver<Command>) {
        let reconnect_option = self.mqttoptions.reconnect_opts();
        let previous_request_stream = self.request_stream(request_rx);
        let mut command_stream = self.command_stream(command_rx);
        let mut network_request_stream = self.network_request_stream(previous_request_stream);

        'reconnection: loop {
            let mqtt_connect_future = self.mqtt_connect();
            let mqtt_connect_deadline = Timeout::new(mqtt_connect_future, Duration::from_secs(30));

            // mqtt connection
            let mut rt = current_thread::Runtime::new().unwrap();
            let framed = match rt.block_on(mqtt_connect_deadline) {
                Ok(framed) => {
                    debug!("Mqtt connection successful!!");
                    self.handle_connection_success();
                    framed
                },
                Err(e) => {
                    error!("Connection error = {:?}", e);
                    self.handle_connection_error(e);
                    if should_reconnect_again(reconnect_option) {
                        continue 'reconnection
                    } else {
                        break 'reconnection
                    }
                }
            };

            let (network_sink, network_stream) = framed.split();
            let network_reply_stream = self.network_reply_stream(network_stream);

            let mqtt_stream = mqttasync::new(network_reply_stream, network_sink, network_request_stream, command_stream);
            let (mqtt_sink, mqtt_stream) = mqtt_stream.split();

            let mqtt_future = mqtt_stream.forward(mqtt_sink);

            // mqtt event loop
            match rt.block_on(mqtt_future) {
                Err(PollError::Network((e, mut r, c))) => {
                    error!("Event loop disconnect. Error = {:?}", e);
                    self.merge_network_request_stream(&mut r);
                    network_request_stream = r;
                    command_stream = c;
                }
                Err(PollError::StreamClosed(mut r, c)) => {
                    error!("Stream closed error");
                    self.merge_network_request_stream(&mut r);
                    network_request_stream = r;
                    command_stream = c;
                }
                _ => panic!("Shouldn't happen")
            }

            if should_reconnect_again(reconnect_option) {
                continue 'reconnection
            } else {
                break 'reconnection
            }
        }
    }


    fn handle_connection_success(&mut self) {
        self.connection_count += 1;

        if self.connection_count == 1 {
            let connection_tx = self.connection_tx.take().unwrap();
            connection_tx.send(Ok(())).unwrap();
        }
    }

    fn handle_connection_error(&mut self, error: timeout::Error<ConnectError>) {
        self.connection_count += 1;

        let error = match error.into_inner() {
            Some(e) => Err(e),
            None => Err(ConnectError::Timeout),
        };

        if self.connection_count == 1 {
            match self.mqttoptions.reconnect_opts() {
                ReconnectOptions::AfterFirstSuccess(_) => {
                    let connection_tx = self.connection_tx.take().unwrap();
                    connection_tx.send(error).unwrap();
                }
                ReconnectOptions::Never => {
                    let connection_tx = self.connection_tx.take().unwrap();
                    connection_tx.send(error).unwrap();
                }
                ReconnectOptions::Always(_) => (),
            }
        }
    }

    /// Resolves dns with blocking API and composes a future
    /// which makes a new tcp or tls connection to the broker.
    /// Note that this doesn't actual connect to the broker
    fn tcp_connect_future(&self) -> impl Future<Item = MqttFramed, Error = ConnectError> {
        let (host, port) = self.mqttoptions.broker_address();
        let connection_method = self.mqttoptions.connection_method();
        let builder = NetworkStream::builder();

        let builder = match connection_method {
            ConnectionMethod::Tls(ca, Some((cert, key))) => builder.add_certificate_authority(&ca).add_client_auth(&cert, &key),
            ConnectionMethod::Tls(ca, None) => builder.add_certificate_authority(&ca),
            ConnectionMethod::Tcp => builder,
        };

        let builder = match self.mqttoptions.proxy() {
            Proxy::None => builder,
            Proxy::HttpConnect(proxy_host, proxy_port, proxy_auth) => {
                builder.set_http_proxy(&proxy_host, proxy_port, &proxy_auth)
            },
        };

        builder.connect(&host, port)
    }

    /// Composes a new future which is a combination of tcp connect + mqtt handshake
    fn mqtt_connect(&self) -> impl Future<Item = MqttFramed, Error = ConnectError> {
        let mqtt_state = self.mqtt_state.clone();
        let tcp_connect_future = self.tcp_connect_future();
        let connect_packet = self.mqtt_state.borrow_mut().handle_outgoing_connect().unwrap();

        tcp_connect_future.and_then(move |framed| {
            let packet = Packet::Connect(connect_packet);
            framed.send(packet).map_err(ConnectError::Io)
        })
            .and_then(|framed| framed.into_future().map_err(|(err, _framed)| ConnectError::Io(err)))
            .and_then(move |(response, framed)| {
                debug!("Mqtt connect response = {:?}", response);
                let mut mqtt_state = mqtt_state.borrow_mut();
                check_and_validate_connack(response, framed, &mut mqtt_state)
            })
    }

    /// Handles all incoming network packets (including sending notifications to user over crossbeam
    /// channel) and creates a stream of packets to send on network
    fn network_reply_stream(&self, network_stream: SplitStream<MqttFramed>) -> impl PacketStream {
        let mqtt_state_in = self.mqtt_state.clone();
        let mqtt_state_out = self.mqtt_state.clone();
        let keep_alive = self.mqttoptions.keep_alive();
        let network_stream = Timeout::new(network_stream, keep_alive);

        // TODO: prevent this clone?
        // cloning crossbeam channel sender every time is a problem according to docs
        let notification_tx = self.notification_tx.clone();
        let network_stream = network_stream.map_err(NetworkError::TimeOut)
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
                                           .or_else(move |e| {
                                               let mut mqtt_state_out = mqtt_state_out.borrow_mut();
                                               handle_stream_error(e, &mut mqtt_state_out)
                                           })
                                           .filter(|reply| should_forward_packet(reply))
                                           .and_then(move |packet| future::ok(packet.into()));

        network_stream.chain(stream::once(Err(NetworkError::NetworkStreamClosed)))
    }

    fn network_request_stream(&mut self, previous_request_stream: impl PacketStream) -> Prepend<impl PacketStream> {
        let mqtt_state = self.mqtt_state.clone();
        let last_session_publishes = mqtt_state.borrow_mut().handle_reconnection();
        previous_request_stream.prepend(last_session_publishes)
    }

    fn merge_network_request_stream(&mut self,
                                    previous_request_stream: &mut Prepend<impl PacketStream>) {
        let mqtt_state = self.mqtt_state.clone();
        let last_session_publishes = mqtt_state.borrow_mut().handle_reconnection();
        previous_request_stream.merge_session(last_session_publishes);
    }

    /// Handles all incoming user and session requests and creates a stream of packets to send
    /// on network
    /// All the remaining packets in the last session (when cleansession = false) will be prepended
    /// to user request stream to ensure that they are handled first. This cleanly handles last
    /// session stray (even if disconnect happens while sending last session data)because we always
    /// get back this stream from reactor after disconnection.
    fn request_stream(&mut self, request: mpsc::Receiver<Request>) -> impl PacketStream {
        let mqtt_state = self.mqtt_state.clone();

        let request_stream = request.map_err(|e| {
                                               error!("User request error = {:?}", e);
                                               NetworkError::Blah
                                           })
                                           .and_then(move |userrequest| {
                                               let mut mqtt_state = mqtt_state.borrow_mut();
                                               validate_userrequest(userrequest, &mut mqtt_state)
                                           });

        let mqtt_state = self.mqtt_state.clone();
        request_stream.and_then(move |packet: Packet| {
            let o = mqtt_state.borrow_mut().handle_outgoing_mqtt_packet(packet);
            future::result(o)
        })
    }

    fn command_stream(&mut self, commands: mpsc::Receiver<Command>) -> impl CommandStream {
        commands.map_err(|e| {
            error!("User request error = {:?}", e);
            NetworkError::Blah
        })
    }
}

fn validate_userrequest(userrequest: Request, mqtt_state: &mut MqttState) -> impl PacketFuture {
    match userrequest {
        Request::Reconnect(mqttoptions) => {
            mqtt_state.opts = mqttoptions;
            future::err(NetworkError::UserReconnect)
        }
        _ => future::ok(userrequest.into()),
    }
}

fn handle_notification(notification: Notification, notification_tx: &Sender<Notification>) {
    match notification {
        Notification::None => (),
        _ => match notification_tx.try_send(notification) {
            Ok(()) => (),
            Err(e) => error!("Notification send failed. Error = {:?}", e),
        },
    }
}

fn handle_stream_error(error: NetworkError, mqtt_state: &mut MqttState) -> impl RequestFuture {
    match error {
        NetworkError::TimeOut(ref e) if e.is_elapsed() => {
            let ping = Packet::Pingreq;
            match mqtt_state.handle_outgoing_mqtt_packet(ping) {
                Ok(_) => future::ok(Request::Ping),
                Err(e) => future::err(e),
            }
        }
        _ => {
            error!("Stream failed. Error = {:?}", error);
            future::err(error)
        }
    }
}

/// Checks if incoming packet is mqtt connack packet. Useful after mqtt
/// connect when we are waiting for connack but not any other packet.
fn check_and_validate_connack(packet: Option<Packet>, framed: MqttFramed, mqtt_state: &mut MqttState) -> impl FramedFuture {
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
                                      p.pkid,
                                      p.payload.len()),

        _ => format!("{:?}", packet),
    }
}

fn should_reconnect_again(reconnect_options: ReconnectOptions) -> bool {
    match reconnect_options {
        ReconnectOptions::AfterFirstSuccess(time) => {
            let time = Duration::from_secs(time);
            thread::sleep(time);
            true
        }
        ReconnectOptions::Always(time) => {
            let time = Duration::from_secs(time);
            thread::sleep(time);
            true
        }
        ReconnectOptions::Never => false
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

type MqttFramed = Framed<NetworkStream, MqttCodec>;

trait PacketStream: Stream<Item = Packet, Error = NetworkError> {}
impl<T> PacketStream for T where T: Stream<Item = Packet, Error = NetworkError> {}

trait CommandStream: Stream<Item = Command, Error = NetworkError> {}
impl<T> CommandStream for T where T: Stream<Item = Command, Error = NetworkError> {}

trait PacketFuture: Future<Item = Packet, Error = NetworkError> {}
impl<T> PacketFuture for T where T: Future<Item = Packet, Error = NetworkError> {}

trait RequestFuture: Future<Item = Request, Error = NetworkError> {}
impl<T> RequestFuture for T where T: Future<Item = Request, Error = NetworkError> {}

trait FramedFuture: Future<Item = MqttFramed, Error = ConnectError> {}
impl<T> FramedFuture for T where T: Future<Item = MqttFramed, Error = ConnectError> {}