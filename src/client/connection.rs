use crate::client::{
    mqttstate::MqttState,
    network::stream::NetworkStream,
    prepend::Prepend,
    Command, Notification, Request, UserHandle,
};
use crate::codec::MqttCodec;
use crate::error::{ConnectError, NetworkError};
use crate::mqttoptions::{ConnectionMethod, MqttOptions, Proxy, ReconnectOptions};
use crossbeam_channel::{self, Sender};
use futures::{
    future::{self, Either},
    stream::{self, poll_fn},
    sync::mpsc::{self, Receiver},
    Async, Future, Poll, Sink, Stream,
};
use mqtt311::Packet;
use std::{cell::RefCell, rc::Rc, thread, time::Duration, io};
use tokio::codec::Framed;
use tokio::prelude::StreamExt;
use tokio::runtime::current_thread::Runtime;
use tokio::timer::{timeout, Timeout};

//  NOTES: Don't use `wait` in eventloop thread even if you
//         are ok with blocking code. It might cause deadlocks
//  https://github.com/tokio-rs/tokio-core/issues/182

pub struct Connection {
    mqtt_state: Rc<RefCell<MqttState>>,
    notification_tx: Sender<Notification>,
    connection_tx: Option<Sender<Result<(), ConnectError>>>,
    connection_count: u32,
    mqttoptions: MqttOptions,
    is_network_enabled: bool,
}

impl Connection {
    /// Takes mqtt options and tries to create initial connection on current thread and handles
    /// connection events in a new thread if the initial connection is successful
    pub fn run(mqttoptions: MqttOptions) -> Result<UserHandle, ConnectError> {
        let (notification_tx, notification_rx) = crossbeam_channel::bounded(mqttoptions.notification_channel_capacity());
        let (request_tx, request_rx) = mpsc::channel::<Request>(mqttoptions.request_channel_capacity());
        let (command_tx, command_rx) = mpsc::channel::<Command>(5);

        let (connection_tx, connection_rx) = crossbeam_channel::bounded(1);
        let reconnect_option = mqttoptions.reconnect_opts();

        // start the network thread to handle all mqtt network io
        thread::spawn(move || {
            let mqtt_state = Rc::new(RefCell::new(MqttState::new(mqttoptions.clone())));
            let mut connection = Connection {
                mqtt_state,
                notification_tx,
                connection_tx: Some(connection_tx),
                connection_count: 0,
                mqttoptions,
                is_network_enabled: true,
            };

            connection.mqtt_eventloop(request_rx, command_rx)
        });

        // return user handle to client to send requests and handle notifications
        let user_handle = UserHandle {
            request_tx,
            command_tx,
            notification_rx,
        };

        match reconnect_option {
            // We need to wait for a successful connection in all cases except for when we always
            // want to reconnect
            ReconnectOptions::AfterFirstSuccess(_) => connection_rx.recv()??,
            ReconnectOptions::Never => connection_rx.recv()??,
            ReconnectOptions::Always(_) => {
                // read the result but ignore it
                let _ = connection_rx.recv()?;
            }
        }

        Ok(user_handle)
    }

    /// Main mqtt event loop. Handles reconnection requests from `connect_or_not` and `mqtt_io`
    fn mqtt_eventloop(&mut self, request_rx: Receiver<Request>, mut command_rx: Receiver<Command>) {
        let network_request_stream = request_rx.map_err(|_| NetworkError::Blah);
        let mut network_request_stream = network_request_stream.prependable();
        let mut command_stream = self.command_stream(command_rx.by_ref());

        'reconnection: loop {
            let mqtt_connect_future = self.mqtt_connect();
            let (runtime, framed) = match self.connect_or_not(mqtt_connect_future) {
                Ok(f) => f,
                Err(true) => continue 'reconnection,
                Err(false) => break 'reconnection,
            };

            let network_request_stream = &mut network_request_stream;
            // Insert previous session. If this is the first connect, the buffer in
            // network_request_stream is empty.
            network_request_stream.insert(self.mqtt_state.borrow_mut().handle_reconnection());

            let mqtt_future = self.mqtt_future(&mut command_stream, network_request_stream, framed);

            match self.mqtt_io(runtime, mqtt_future) {
                Err(true) => continue 'reconnection,
                Err(false) => break 'reconnection,
                Ok(_v) => continue 'reconnection,
            }
        }
    }


    /// Makes a blocking mqtt connection an returns framed and reactor which created
    /// the connection when `is_network_enabled` flag is set true
    fn connect_or_not(&mut self, mqtt_connect_future: impl Future<Item = MqttFramed, Error = ConnectError>) -> Result<(Runtime, Option<MqttFramed>), bool> {
        let mut rt = Runtime::new().unwrap();
        let timeout = Duration::from_secs(30);
        let mqtt_connect_deadline = Timeout::new(mqtt_connect_future, timeout);

        if !self.is_network_enabled {
            return Ok((rt, None));
        }

        let framed = match rt.block_on(mqtt_connect_deadline) {
            Ok(framed) => {
                info!("Mqtt connection successful!!");
                self.handle_connection_success();
                framed
            }
            Err(e) => {
                error!("Connection error = {:?}", e);
                self.handle_connection_error(e);
                return Err(self.should_reconnect_again());
            }
        };

        Ok((rt, Some(framed)))
    }

    /// Tells whether eventloop should try to reconnect or not based
    /// user reconnection configuration
    fn should_reconnect_again(&self) -> bool {
        let reconnect_options = self.mqttoptions.reconnect_opts();

        match reconnect_options {
            ReconnectOptions::Always(time) => {
                let time = Duration::from_secs(time);
                thread::sleep(time);
                true
            }
            ReconnectOptions::AfterFirstSuccess(time) => {
                // should reconnect only if initial connection was successful
                let reconnect = self.connection_count > 0;
                if reconnect {
                    let time = Duration::from_secs(time);
                    thread::sleep(time);
                }

                reconnect
            }
            ReconnectOptions::Never => false,
        }
    }

    /// Ananlyses the eventloop return cases and decides if a reconnection is necessary
    /// or not based on user commands like shutdown, disconnect and reconnect and reconnect
    /// options.
    /// Err(true) -> Reconnect
    /// Err(false) -> Don't reconnect
    fn mqtt_io(&mut self, mut runtime: Runtime, mqtt_future: impl Future<Item = (), Error = NetworkError>) -> Result<(), bool> {
        let o = runtime.block_on(mqtt_future);
        if let Err(e) = self.notification_tx.try_send(Notification::Disconnection) {
            error!("Notification failure. Error = {:?}", e);
        }

        if let Err(e) = o {
            debug!("Eventloop stopped with error. {:?}", e);

            return match e {
                NetworkError::UserDisconnect => {
                    self.is_network_enabled = false;
                    Err(true)
                }
                NetworkError::UserReconnect => {
                    self.is_network_enabled = true;
                    Err(true)
                }
                NetworkError::NetworkStreamClosed if self.mqtt_state.borrow().is_disconnecting() => {
                    self.is_network_enabled = false;
                    Err(false)
                }
                NetworkError::NetworkStreamClosed => {
                    self.is_network_enabled = true;
                    Err(self.should_reconnect_again())
                }
                _ => {
                    self.is_network_enabled = true;
                    Err(self.should_reconnect_again())
                }
            }
        }

        if let Ok(_v) = o {
            debug!("Eventloop stopped without error");
            return Err(self.should_reconnect_again())
        }

        Ok(())
    }

    /// Applies throttling and inflight limiting based on user configuration and returns
    /// a statful mqtt event loop future to be run on the reactor. The returned future also
    /// conditionally enable/disables network functionality based on the current `framed` state
    fn mqtt_future(&mut self, command_stream: impl PacketStream, network_request_stream: impl RequestStream, framed: Option<Framed<NetworkStream, MqttCodec>>) -> impl Future<Item = (), Error = NetworkError> {
        // convert a request stream to request packet stream after filtering
        // unnecessary requests and apply inflight limiting and rate limiting
        // note: make sure that the order remains (inflight, rate, request handling)
        // or else inflight limiting might face off by one bugs like progressing after
        // receiving 2 acks insteam of 1 ack
        let network_request_stream = self.inflight_limited_request_stream(network_request_stream);
        let network_request_stream = self.throttled_network_stream(network_request_stream);
        let network_request_stream = self.user_requests(network_request_stream);
        let network_request_stream = network_request_stream.and_then(move |packet| future::ok(packet.into()));

        // check if the network is enabled and create a future
        let network = match framed {
            Some(f) => {
                let (network_sink, network_stream) = f.split();
                let network_sink = network_sink.sink_map_err(NetworkError::Io);
                let network_reply_stream = self.network_reply_stream(network_stream);
                Ok((network_reply_stream, network_sink, command_stream))
            }
            None => Err(command_stream),
        };

        match network {
            Ok((network_reply_stream, network_sink, command_stream)) => {
                // convert rquests to packets
                let network_reply_stream = network_reply_stream.map(|r| r.into());
                let network_stream = network_reply_stream.select(network_request_stream);
                let stream = command_stream.select(network_stream);
                let f = stream.forward(network_sink).map(|_| ());
                Either::A(f)
            }
            Err(command_stream) => {
                let dummy_sink = BlackHole;
                let f = command_stream.forward(dummy_sink).map(|_| ());
                Either::B(f)
            }
        }
    }

    /// Sends connection status on blocked connections status call in `run`
    fn handle_connection_success(&mut self) {
        if self.connection_count == 0 {
            let connection_tx = self.connection_tx.take().unwrap();
            connection_tx.try_send(Ok(())).unwrap();
        } else {
            let _ = self.notification_tx.try_send(Notification::Reconnection);
        }

        self.connection_count += 1;
    }

    /// Sends connection status on blocked connections status call in `run`
    /// TODO: Combine both
    fn handle_connection_error(&mut self, error: timeout::Error<ConnectError>) {
        let error = match error.into_inner() {
            Some(e) => Err(e),
            None => Err(ConnectError::Timeout),
        };

        if self.connection_count == 0 {
            let connection_tx = self.connection_tx.take().unwrap();
            connection_tx.try_send(error).unwrap();
        }
    }

    /// Resolves dns with blocking API and composes a future which makes a new tcp
    /// or tls connection to the broker. Note that this doesn't actual connect to the
    /// broker
    fn tcp_connect_future(&self) -> impl Future<Item = MqttFramed, Error = ConnectError> {
        let (host, port) = self.mqttoptions.broker_address();
        let connection_method = self.mqttoptions.connection_method();
        let proxy = self.mqttoptions.proxy();

        let builder = NetworkStream::builder();

        let builder = match connection_method {
            ConnectionMethod::Tls { ca, cert_and_key, alpn } => {
                let builder = builder.add_certificate_authority(&ca).add_alpn_protocols(&alpn);
                if let Some((ref cert, ref key)) = cert_and_key {
                    builder.add_client_auth(cert, key)
                } else {
                    builder
                }
            },
            ConnectionMethod::Tcp => builder,
        };

        let builder = match proxy {
            Proxy::None => builder,
            Proxy::HttpConnect(proxy_host, proxy_port, key, expiry) => {
                let id = self.mqttoptions.client_id();
                builder.set_http_proxy(&id, &proxy_host, proxy_port, &key, expiry)
            }
        };

        builder.connect(&host, port)
    }

    /// Composes a new future which is a combination of tcp connect + mqtt handshake
    fn mqtt_connect(&self) -> impl Future<Item = MqttFramed, Error = ConnectError> {
        let mqtt_state = self.mqtt_state.clone();
        let tcp_connect_future = self.tcp_connect_future();
        let connect_packet = self.mqtt_state.borrow_mut().handle_outgoing_connect().unwrap();

        tcp_connect_future
            .and_then(move |framed| {
                let packet = Packet::Connect(connect_packet);
                framed.send(packet).map_err(ConnectError::Io)
            })
            .and_then(|framed| framed.into_future().map_err(|(err, _framed)| ConnectError::Io(err)))
            .and_then(move |(response, framed)| {
                info!("Mqtt connect response = {:?}", response);
                let mut mqtt_state = mqtt_state.borrow_mut();
                check_and_validate_connack(response, framed, &mut mqtt_state)
            })
    }

    /// Handles all incoming network packets (including sending notifications to user over crossbeam
    /// channel) and creates a stream of packets to send on network
    fn network_reply_stream(&self, network_stream: impl Stream<Item = Packet, Error = io::Error>) -> impl Stream<Item = Request, Error = NetworkError> {
        let mqtt_state = self.mqtt_state.clone();
        let mqtt_state_ping = self.mqtt_state.clone();

        let keep_alive = self.mqttoptions.keep_alive();
        let notification_tx = self.notification_tx.clone();

        let network_stream = network_stream.timeout(keep_alive)
            .or_else(move |e| {
                debug!("Idle network incoming timeout");
                let mut mqtt_state = mqtt_state_ping.borrow_mut();
                handle_incoming_stream_timeout_error(e, &mut mqtt_state)
            })
            .and_then(move |packet| {
                debug!("Incoming packet = {:?}", packet_info(&packet));
                let reply = mqtt_state.borrow_mut().handle_incoming_mqtt_packet(packet);
                future::result(reply)
            })
            .and_then(move |(notification, reply)| {
                handle_notification_and_reply(&notification_tx, notification, reply)
            })
            .filter(|reply| should_forward_packet(reply));

        let network_reply_stream = network_stream.chain(stream::once(Err(NetworkError::NetworkStreamClosed)));
        let mqtt_state = self.mqtt_state.clone();

        // when there are no outgoing replies, timeout should check if a ping is
        // necessary. E.g If there are only qos0 incoming publishes,
        // incoming network timeout will never trigger the ping. But broker needs a
        // ping when there are no outgoing packets. This timeout will take care of that
        // When network is completely idle, incoming network idle ping triggers first
        // and this timeout doesn't happen
        // When there are only qos0 incoming publishes, this timeout alone triggers
        let timeout = keep_alive + Duration::from_millis(500);
        network_reply_stream.timeout(timeout)
            .or_else(move |e| {
                debug!("Idle network reply timeout");
                let mut mqtt_state = mqtt_state.borrow_mut();
                handle_outgoing_stream_timeout_error(e, &mut mqtt_state)
            })
            .filter(|reply| should_forward_packet(reply))
    }

    /// Handles all incoming user and session requests and creates a stream of packets to send
    /// on network
    /// All the remaining packets in the last session (when cleansession = false) will be prepended
    /// to user request stream to ensure that they are handled first. This cleanly handles last
    /// session stray (even if disconnect happens while sending last session data)because we always
    /// get back this stream from reactor after disconnection.
    fn user_requests(&mut self, request: impl RequestStream) -> impl RequestStream {
        // process user requests and convert them to network packets
        let mqtt_state = self.mqtt_state.clone();
        let request_stream = request
            .map_err(|e| {
                error!("User request error = {:?}", e);
                NetworkError::Blah
            })
            .and_then(move |userrequest| {
                let mut mqtt_state = mqtt_state.borrow_mut();
                validate_userrequest(userrequest, &mut mqtt_state)
            });

        let mqtt_state = self.mqtt_state.clone();
        request_stream.and_then(move |packet: Packet| {
            let mut mqtt_state = mqtt_state.borrow_mut();
            let o = mqtt_state.handle_outgoing_mqtt_packet(packet);
            future::result(o)
        })
    }

    // Apply outgoing queue limit (in flights) by answering stream poll with not ready if queue is full
    // by returning NotReady.
    fn inflight_limited_request_stream(&self, requests: impl RequestStream) -> impl RequestStream {
        let mqtt_state = self.mqtt_state.clone();
        let in_flight = self.mqttoptions.inflight();
        let mut stream = requests.peekable();

        // don't read anything from the user request stream if current queue length
        // is >= max inflight messages. Select's poll will also call this poll when ever
        // there's an incoming ack which ensures the progress
        // TODO: Understand poll_fn wakeups
        // https://play.rust-lang.org/?version=stable&mode=debug&edition=2018&gist=fcf42c86eb819053fe9eeaa1a2f457e6
        poll_fn(move || -> Poll<Option<Request>, NetworkError> {
            let current_queue_len = mqtt_state.borrow().publish_queue_len();
            if current_queue_len >= in_flight {
                match stream.peek() {
                    Err(_) => stream.poll(),
                    _ => Ok(Async::NotReady),
                }
            } else {
                stream.poll()
            }
        })
    }

    /// Apply throttling if configured
    fn throttled_network_stream(&mut self, requests: impl RequestStream) -> impl RequestStream {
        if let Some(rate) = self.mqttoptions.throttle() {
            let duration = Duration::from_nanos(1_000_000_000 / rate);
            let throttled = requests.throttle(duration).map_err(|_| NetworkError::Throttle);
            EitherStream::A(throttled)
        } else {
            EitherStream::B(requests)
        }
    }

    /// Convert commands to errors
    fn command_stream<'a>(&mut self, commands: &'a mut mpsc::Receiver<Command>) -> impl PacketStream + 'a {
        // process user commands and raise appropriate error to the event loop
        commands
            .or_else(|_err| Err(NetworkError::Blah))
            .and_then(|usercommand| match usercommand {
                Command::Pause => Err(NetworkError::UserDisconnect),
                Command::Resume => Err(NetworkError::UserReconnect),
            })
    }
}

fn handle_notification_and_reply(notification_tx: &Sender<Notification>, notification: Notification, reply: Request) -> impl Future<Item = Request, Error = NetworkError> {
    match notification {
        Notification::None => future::ok(reply),
        _ => match notification_tx.try_send(notification) {
            Ok(()) => {
                future::ok(reply)
            }
            Err(e) => {
                error!("Notification send failed. Error = {:?}", e);
                future::err(NetworkError::ReceiverCatchup)
            }
        }
    }
}

/// Checks if a ping is necessary based on timeout error
fn handle_incoming_stream_timeout_error(error: timeout::Error<io::Error>, mqtt_state: &mut MqttState) -> impl Future<Item = Packet, Error = NetworkError> {
    // check if a ping to the broker is necessary
    let out = mqtt_state.handle_outgoing_ping();
    future::err(error).or_else(move |e| {
        if e.is_elapsed() {
            match out {
                Ok(_) => future::ok(Packet::Pingreq),
                Err(e) => future::err(e),
            }
        } else {
            future::err(e.into_inner().unwrap().into())
        }
    })
}

/// Checks if a ping is necessary based on timeout error
fn handle_outgoing_stream_timeout_error(error: timeout::Error<NetworkError>, mqtt_state: &mut MqttState) -> impl Future<Item = Request, Error = NetworkError> {
    // check if a ping to the broker is necessary
    let out = mqtt_state.handle_outgoing_ping();
    future::err(error).or_else(move |e| {
        if e.is_elapsed() {
            match out {
                Ok(true) => future::ok(Request::OutgoingIdlePing),
                Ok(false) => future::ok(Request::None),
                Err(e) => future::err(e),
            }
        } else {
            future::err(e.into_inner().unwrap().into())
        }
    })
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

/// Checks if incoming packet is mqtt connack packet. Useful after mqtt
/// connect when we are waiting for connack but not any other packet.
fn check_and_validate_connack(packet: Option<Packet>, framed: MqttFramed, mqtt_state: &mut MqttState) -> impl Future<Item = MqttFramed, Error = ConnectError> {
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
        Packet::Publish(p) => format!(
            "topic = {}, \
             qos = {:?}, \
             pkid = {:?}, \
             payload size = {:?} bytes",
            p.topic_name,
            p.qos,
            p.pkid,
            p.payload.len()
        ),

        _ => format!("{:?}", packet),
    }
}

fn _request_info(packet: &Request) -> String {
    match packet {
        Request::Publish(p) => format!(
            "topic = {}, \
             qos = {:?}, \
             pkid = {:?}, \
             payload size = {:?} bytes",
            p.topic_name,
            p.qos,
            p.pkid,
            p.payload.len()
        ),

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
            Request::IncomingIdlePing => Packet::Pingreq,
            Request::OutgoingIdlePing => Packet::Pingreq,
            Request::Disconnect => Packet::Disconnect,
            Request::Subscribe(subscribe) => Packet::Subscribe(subscribe),
            Request::Unsubscribe(unsubscribe) => Packet::Unsubscribe(unsubscribe),
            _ => unimplemented!(),
        }
    }
}


type MqttFramed = Framed<NetworkStream, MqttCodec>;

trait PacketStream: Stream<Item = Packet, Error = NetworkError> {}
impl<T> PacketStream for T where T: Stream<Item = Packet, Error = NetworkError> {}

trait PacketSink: Sink<SinkItem = Packet, SinkError = NetworkError> {}
impl<T> PacketSink for T where T: Sink<SinkItem = Packet, SinkError = NetworkError> {}

trait CommandStream: Stream<Item = Command, Error = NetworkError> {}
impl<T> CommandStream for T where T: Stream<Item = Command, Error = NetworkError> {}

trait RequestStream: Stream<Item = Request, Error = NetworkError> {}
impl<T> RequestStream for T where T: Stream<Item = Request, Error = NetworkError> {}

trait PacketFuture: Future<Item = Packet, Error = NetworkError> {}
impl<T> PacketFuture for T where T: Future<Item = Packet, Error = NetworkError> {}

trait RequestFuture: Future<Item = Request, Error = NetworkError> {}
impl<T> RequestFuture for T where T: Future<Item = Request, Error = NetworkError> {}

// TODO: Remove if impl Either for Stream is backported to futures 0.1
// See https://github.com/rust-lang-nursery/futures-rs/issues/614
#[derive(Debug)]
pub enum EitherStream<A, B> {
    /// First branch of the type
    A(A),
    /// Second branch of the type
    B(B),
}

impl<A, B> Stream for EitherStream<A, B>
where
    A: Stream,
    B: Stream<Item = A::Item, Error = A::Error>,
{
    type Item = A::Item;
    type Error = A::Error;

    fn poll(&mut self) -> Poll<Option<A::Item>, A::Error> {
        match *self {
            EitherStream::A(ref mut a) => a.poll(),
            EitherStream::B(ref mut b) => b.poll(),
        }
    }
}

use futures::{AsyncSink, StartSend};

#[derive(Debug)]
struct BlackHole;

impl Sink for BlackHole {
    type SinkItem = Packet;
    type SinkError = NetworkError;

    fn start_send(&mut self, _item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        Ok(AsyncSink::Ready)
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        Ok(Async::Ready(()))
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        Ok(Async::Ready(()))
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;
    use tokio::timer::DelayQueue;
    use mqtt311::PacketIdentifier;
    #[cfg(target_os = "linux")] use crate::client::Request;
    use crate::client::Notification;
    use super::{Connection, MqttOptions, MqttState, NetworkError, ConnectError, ReconnectOptions};
    use super::MqttFramed;
    use futures::{
        future,
        stream::Stream,
    };
    use mqtt311::Packet;
    use mqtt311::Publish;
    use mqtt311::QoS;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::io;
    #[cfg(target_os = "linux")] use std::time::Instant;
    use std::thread;
    use tokio::runtime::current_thread::Runtime;

    struct UserHandle {
        notification_rx: crossbeam_channel::Receiver<Notification>,
        connection_rx: crossbeam_channel::Receiver<Result<(), ConnectError>>
    }

    fn mock_mqtt_connection(mqttoptions: MqttOptions, mqtt_state: MqttState) -> (Connection, UserHandle, Runtime) {
        let (connection_tx, connection_rx) = crossbeam_channel::bounded(1);
        let (notification_tx, notification_rx) = crossbeam_channel::bounded(10);

        let mqtt_state = Rc::new(RefCell::new(mqtt_state));
        let connection = Connection {
            mqtt_state,
            notification_tx,
            connection_tx: Some(connection_tx),
            connection_count: 0,
            mqttoptions,
            is_network_enabled: true,
        };

        let userhandle = UserHandle {
            notification_rx,
            connection_rx
        };

        let runtime = Runtime::new().unwrap();
        (connection, userhandle, runtime)
    }

    #[cfg(target_os = "linux")]
    fn user_requests(delay: Duration) -> impl Stream<Item = Request, Error = NetworkError> {
        let mut requests = DelayQueue::new();

        for i in 1..=100 {
            let publish = Publish {
                dup: false,
                qos: QoS::AtLeastOnce,
                retain: false,
                pkid: None,
                topic_name: "hello/world".to_owned(),
                payload: Arc::new(vec![1, 2, 3]),
            };
            let request = Request::Publish(publish.clone());
            requests.insert(request, i * delay);
        }

        requests.map(|v| {
            v.into_inner()
        }).map_err(|e| NetworkError::Timer(e))
    }

    fn network_incoming_publishes(delay: Duration, count: u32) -> impl Stream<Item = Packet, Error = io::Error> {
        let mut publishes = DelayQueue::new();

        for i in 1..=count {
            let publish = Publish {
                dup: false,
                qos: QoS::AtLeastOnce,
                retain: false,
                pkid: Some(PacketIdentifier(i as u16)),
                topic_name: "hello/world".to_owned(),
                payload: Arc::new(vec![1, 2, 3]),
            };

            publishes.insert(Packet::Publish(publish), i * delay);
        }

        publishes.map(|v| {
            v.into_inner()
        }).map_err(|_e| {
            io::Error::new(io::ErrorKind::Other, "Timer error")
        })
    }

    #[cfg(target_os = "linux")]
    fn network_incoming_acks(delay: Duration) -> impl Stream<Item = Packet, Error = io::Error> {
        let mut acks = DelayQueue::new();

        for i in 1..=100 {
            acks.insert(Packet::Puback(PacketIdentifier(i as u16)), i * delay);
        }

        acks.map(|v| {
            v.into_inner()
        }).map_err(|_e| {
            io::Error::new(io::ErrorKind::Other, "Timer error")
        })
    }

    #[test]
    fn run_should_raise_connection_errors_based_on_reconnection_options() {
        // local broker isn't running. Should result in connection errors
        let reconnect_opt = ReconnectOptions::Never;
        let mqttoptions = MqttOptions::new("mqtt-io-test", "localhost", 1883).set_reconnect_opts(reconnect_opt);

        // error in `never connect` case
        let o = Connection::run(mqttoptions);
        assert!(o.is_err());

        let reconnect_opt = ReconnectOptions::AfterFirstSuccess(10);
        let mqttoptions = MqttOptions::new("mqtt-io-test", "localhost", 1883).set_reconnect_opts(reconnect_opt);

        // error in `after first success` case
        let o = Connection::run(mqttoptions);
        assert!(o.is_err());

        // no error in `always` case
        let reconnect_opt = ReconnectOptions::Always(10);
        let mqttoptions = MqttOptions::new("mqtt-io-test", "localhost", 1883).set_reconnect_opts(reconnect_opt);

        let o = Connection::run(mqttoptions);
        assert!(o.is_ok());
    }

    #[test]
    fn connect_or_not_returns_correct_reconnection_behaviour_in_always_reconnect_mode() {
        let reconnect_opt = ReconnectOptions::Always(10);
        let mqttoptions = MqttOptions::new("mqtt-io-test", "localhost", 1883).set_reconnect_opts(reconnect_opt);
        let mqtt_state = MqttState::new(mqttoptions.clone());

        // disconnections should take user reconnection options into consideration
        let (mut connection, userhandle, _runtime) = mock_mqtt_connection(mqttoptions.clone(), mqtt_state);
        let ioerror = io::Error::new(io::ErrorKind::Other, "oh no!");
        let connect_future = future::err::<MqttFramed, _>(ConnectError::Io(ioerror));

        // results in an error but continues reconnection
        match connection.connect_or_not(connect_future) {
            Err(true) => (),
            _ => panic!("Should return reconnect = true")
        }
        assert!(userhandle.connection_rx.recv().unwrap().is_err());
    }

    #[test]
    fn connect_or_not_returns_dontreconnect_in_afterfirstsuccess_mode_during_first_failure() {
        // first connection
        let reconnect_opt = ReconnectOptions::AfterFirstSuccess(5);
        let mqttoptions = MqttOptions::new("mqtt-io-test", "localhost", 1883).set_reconnect_opts(reconnect_opt);
        let mqtt_state = MqttState::new(mqttoptions.clone());

        // disconnections should take user reconnection options into consideration
        let (mut connection, userhandle, _runtime) = mock_mqtt_connection(mqttoptions.clone(), mqtt_state);
        let ioerror = io::Error::new(io::ErrorKind::Other, "oh no!");
        let connect_future = future::err::<MqttFramed, _>(ConnectError::Io(ioerror));

        // results in an error and reconnection = false during 1st reconnection
        match connection.connect_or_not(connect_future) {
            Err(false) => (),
            Err(true) => panic!("Should return reconnect = false"),
            Ok(_) => panic!("not possible")
        }
        assert!(userhandle.connection_rx.recv().unwrap().is_err());
    }

    #[test]
    fn connect_or_not_returns_reconnect_in_afterfirstsuccess_mode_during_second_failure() {
        // first connection
        let reconnect_opt = ReconnectOptions::AfterFirstSuccess(3);
        let mqttoptions = MqttOptions::new("mqtt-io-test", "localhost", 1883).set_reconnect_opts(reconnect_opt);
        let mqtt_state = MqttState::new(mqttoptions.clone());

        // disconnections should take user reconnection options into consideration
        let (mut connection, _userhandle, _runtime) = mock_mqtt_connection(mqttoptions.clone(), mqtt_state);
        connection.connection_count = 1;
        let ioerror = io::Error::new(io::ErrorKind::Other, "oh no!");
        let connect_future = future::err::<MqttFramed, _>(ConnectError::Io(ioerror));

        // results in an error and reconnection = false during 1st reconnection
        match connection.connect_or_not(connect_future) {
            Err(true) => (),
            Err(false) => panic!("Should return reconnect = true"),
            Ok(_) => panic!("not possible")
        }
    }

    #[test]
    fn mqtt_io_returns_correct_reconnection_behaviour() {
        let reconnect_opt = ReconnectOptions::Always(10);
        let mqttoptions = MqttOptions::new("mqtt-io-test", "localhost", 1883).set_reconnect_opts(reconnect_opt);
        let mqtt_state = MqttState::new(mqttoptions.clone());

        // disconnections should take user reconnection options into consideration
        let (mut connection, _userhandle, runtime) = mock_mqtt_connection(mqttoptions.clone(), mqtt_state);
        let network_future = future::err::<(), _>(NetworkError::NetworkStreamClosed);
        let out = connection.mqtt_io(runtime, network_future);
        assert_eq!(out, Err(true));

        let mqtt_state = MqttState::new(mqttoptions.clone());
        // user shutdown should not take reconnection options into consideration
        let (mut connection, _userhandle, runtime) = mock_mqtt_connection(mqttoptions, mqtt_state);
        connection.mqtt_state.borrow_mut().handle_outgoing_disconnect().unwrap();
        let network_future = future::err::<(), _>(NetworkError::NetworkStreamClosed);
        let out = connection.mqtt_io(runtime, network_future);
        assert_eq!(out, Err(false));
    }

    #[cfg(target_os = "linux")]
    // incoming puback at second 1 and pingresp at periodic intervals
    fn network_incoming_pingresps() -> impl Stream<Item = Packet, Error = io::Error> {
        let mut acks = DelayQueue::new();

        let publish = Publish {
            dup: false,
            qos: QoS::AtMostOnce,
            retain: false,
            pkid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        acks.insert( Packet::Publish(publish), Duration::from_secs(2));
        // out idle ping at 5000 + 500 (out ping delay wrt to keep alive)
        acks.insert(Packet::Pingresp, Duration::from_millis(5510));
        // in idle ping at 10520
        acks.insert(Packet::Pingresp, Duration::from_millis(10520));
        // in idle ping at 15530
        acks.insert(Packet::Pingresp, Duration::from_millis(15530));

        acks.map(|v| {
            v.into_inner()
        }).map_err(|_e| {
            io::Error::new(io::ErrorKind::Other, "Timer error")
        })
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reply_stream_triggers_pings_on_time() {
        let mqttoptions = MqttOptions::default().set_keep_alive(5);
        let mqtt_state = MqttState::new(mqttoptions.clone());

        let (connection, _userhandle, mut runtime) = mock_mqtt_connection(mqttoptions, mqtt_state);
        let network_reply_stream = network_incoming_pingresps();
        let network_reply_stream = connection.network_reply_stream(network_reply_stream);

        let start = Instant::now();

        // incoming publish at second 1,
        // so pingreq should happen at second 6, 12 and 16
        let network_future = network_reply_stream.fold(1, |mut count, packet| {
            let elapsed = start.elapsed().as_millis();
            println!("Packet = {:?}, Elapsed = {:?}", packet, elapsed);
            match packet {
                // incoming publish at 2000. (in idle, out idle) = (7000, 5000 + 500) ---> out idle ping at 5500
                Request::OutgoingIdlePing if count == 1 =>  assert!(elapsed > 5500 && elapsed < 5700),
                // ping resp at 5510. (in idle, out idle) = (10510, 5500 + 5500) ---> in idle ping at 10510
                Request::IncomingIdlePing if count == 2 =>  assert!(elapsed > 10510 && elapsed < 10700),
                // ping resp at 10520. (in idl, out idle) = (15520, 10510 + 5500) ---> in idle ping at 15520
                Request::IncomingIdlePing if count == 3 =>  assert!(elapsed > 15520 && elapsed < 15700),
                _ => panic!("Expecting publish or ping")
            }

            count += 1;
            future::ok::<_, NetworkError>(count)
        });

        match runtime.block_on(network_future) {
            Err(NetworkError::NetworkStreamClosed) | Ok(_) => (),
            Err(e) => panic!("Error = {:?}", e)
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn throttled_stream_operates_at_specified_rate() {
        let mqttoptions = MqttOptions::default().set_throttle(5);
        let mqtt_state = MqttState::new(mqttoptions.clone());

        let (mut connection, _userhandle, mut runtime) = mock_mqtt_connection(mqttoptions, mqtt_state);

        // note: maintain order similar to mqtt_future()
        // generates 100 user requests
        let user_request_stream = user_requests(Duration::from_millis(1));
        let user_request_stream = connection.throttled_network_stream(user_request_stream);
        let user_request_stream = connection.user_requests(user_request_stream);
        let user_request_stream = user_request_stream.and_then(move |packet| future::ok(packet.into()));

        let f = user_request_stream.fold(Instant::now(), |last, v: Packet| {
            // println!("outgoing = {:?}", v);
            let now = Instant::now();

            if let Packet::Publish(Publish{pkid, ..}) = v {
                if pkid.unwrap() > PacketIdentifier(1) {
                    let elapsed = (now - last).as_millis();
                    dbg!(elapsed);
                    assert!(elapsed > 190 && elapsed < 220)
                }
            }  
            
            future::ok::<_, NetworkError>(now)
        });
        
        let _ = runtime.block_on(f);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn requests_should_block_during_max_in_flight_messages() {
        let mqttoptions = MqttOptions::default().set_inflight(50);
        let mqtt_state = MqttState::new(mqttoptions.clone());

        let (mut connection, _userhandle, mut runtime) = mock_mqtt_connection(mqttoptions, mqtt_state);
        
        // note: maintain order similar to mqtt_future()
        // generates 100 user requests
        let user_request_stream = user_requests(Duration::from_millis(1));
        let user_request_stream = connection.inflight_limited_request_stream(user_request_stream);
        let user_request_stream = connection.user_requests(user_request_stream);
        let user_request_stream = user_request_stream.map(|r| r.into());

        // generates 100 acks
        let network_reply_stream = network_incoming_acks(Duration::from_millis(200));
        let network_reply_stream = connection.network_reply_stream(network_reply_stream);
        let network_reply_stream = network_reply_stream.map(|r| r.into());
        let network_stream = network_reply_stream.select(user_request_stream);
        let network_stream = network_stream.fold(Instant::now(), |last, v| {
            // println!("outgoing = {:?}", v);
            let now = Instant::now();
            
            if let Packet::Publish(Publish{pkid, ..}) = v {
                if pkid.unwrap() > PacketIdentifier(51) {
                    let elapsed = (now - last).as_millis();
                    dbg!(elapsed);
                    assert!(elapsed > 190 && elapsed < 220)
                }
            }
            
            future::ok::<_, NetworkError>(now)
        });
        let _ = runtime.block_on(network_stream);
    }

    #[test]
    fn reply_stream_results_in_an_error_when_notification_receiver_doesnt_catchup() {
        let mqttoptions = MqttOptions::default().set_inflight(50);
        let mqtt_state = MqttState::new(mqttoptions.clone());

        let (connection, _userhandle, mut runtime) = mock_mqtt_connection(mqttoptions, mqtt_state);
        let network_reply_stream = network_incoming_publishes(Duration::from_millis(100), 11);
        let network_reply_stream = connection.network_reply_stream(network_reply_stream);

        let network_future = network_reply_stream.for_each(|_v| {
            // println!("Incoming = {:?}", v);
            future::ok(())
        });

        match runtime.block_on(network_future) {
            Err(NetworkError::ReceiverCatchup) => (),
            _ => panic!("Should result in receiver catchup error")
        }
    }

    #[test]
    fn connection_success_and_disconnections_should_put_state_change_events_on_notifications() {
        let mqttoptions = MqttOptions::default().set_inflight(50);
        let mqtt_state = MqttState::new(mqttoptions.clone());

        let (mut connection, userhandle, runtime) = mock_mqtt_connection(mqttoptions, mqtt_state);
        connection.handle_connection_success();

        thread::spawn(move || {
            for (count, notification) in userhandle.notification_rx.iter().enumerate() {
                match notification {
                    Notification::Reconnection if count == 0 => (),
                    Notification::Disconnection if count == 21 => (),
                    Notification::Publish(_) if count != 0 || count != 21 => (),
                    n => panic!("Not expected notification {:?}", n)
                }
            }
        });

        // puts connection success event on the notifaction channel
        connection.handle_connection_success();
        let network_reply_stream = network_incoming_publishes(Duration::from_millis(100), 20);
        // end of the stream will simulate server disconnection
        let network_reply_stream = connection.network_reply_stream(network_reply_stream);
        let network_future = network_reply_stream.for_each(|_v| {
            future::ok(())
        });

        let _ = connection.mqtt_io(runtime, network_future);
    }
}


// fn print_last_session_state(
//     prepend: &mut Prepend<impl RequestStream>,
//     state: &MqttState) {
//         let last_session_data = prepend.session.iter().map(|request| {
//             match request {
//                 Request::Publish(publish) => publish.pkid,
//                 _ => None
//             }
//         }).collect::<VecDeque<Option<PacketIdentifier>>>();

//         debug!("{:?}", last_session_data);
//         debug!("{:?}", state.publish_queue_len())
// }

// NOTE: We need to use same reactor across threads because io resources (framed) will
//       bind to reactor lazily.
//       Results in `reactor gone` error if `framed` is used again with a new recator
