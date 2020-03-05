use crate::{
    client::{
        mqttstate::MqttState,
        network::stream::NetworkStream,
        prepend::Prepend,
        Command,
        Notification,
        Request,
        UserHandle,
    },
    codec::MqttCodec,
    error::{ConnectError, NetworkError},
    mqttoptions::{MqttOptions, Proxy, ReconnectOptions},
};
use futures::{
    channel::{
        mpsc::{self, Receiver, Sender},
        oneshot,
    },
    future::Either,
    lock::Mutex,
    sink::SinkExt,
    stream::{self, TryStreamExt},
    Future,
    Sink,
    Stream,
    StreamExt,
};
use mqtt311::Packet;
use std::{
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::time;
use tokio_util::codec::Framed;

//  NOTES: Don't use `wait` in eventloop thread even if you
//         are ok with blocking code. It might cause deadlocks
//  https://github.com/tokio-rs/tokio-core/issues/182

pub struct Connection {
    mqtt_state: Arc<Mutex<MqttState>>,
    notification_tx: Sender<Notification>,
    connection_tx: Option<oneshot::Sender<Result<(), ConnectError>>>,
    connection_count: u32,
    mqttoptions: MqttOptions,
    is_network_enabled: bool,
}

impl Connection {
    /// Takes mqtt options and tries to create initial connection on current thread and handles
    /// connection events in a new thread if the initial connection is successful
    pub async fn run(mqttoptions: MqttOptions) -> Result<UserHandle, ConnectError> {
        let (notification_tx, notification_rx) = mpsc::channel(mqttoptions.notification_channel_capacity());
        let (request_tx, request_rx) = mpsc::channel::<Request>(mqttoptions.request_channel_capacity());
        let (command_tx, command_rx) = mpsc::channel::<Command>(5);

        let (connection_tx, connection_rx) = oneshot::channel();
        let reconnect_option = mqttoptions.reconnect_opts();

        // start the network thread to handle all mqtt network io
        let _ = tokio::spawn(Self::eventloop_future(
            mqttoptions.clone(),
            notification_tx,
            connection_tx,
            request_rx,
            command_rx,
        ));

        // return user handle to client to send requests and handle notifications
        let user_handle = UserHandle {
            request_tx,
            command_tx,
            notification_rx,
        };

        match reconnect_option {
            // We need to wait for a successful connection in all cases except for when we always
            // want to reconnect
            ReconnectOptions::AfterFirstSuccess(_) => connection_rx.await??,
            ReconnectOptions::Never => connection_rx.await??,
            ReconnectOptions::Always(_) => {
                // read the result but ignore it
                let _ = connection_rx.await?;
            }
        }

        Ok(user_handle)
    }

    async fn eventloop_future(
        mqttoptions: MqttOptions,
        notification_tx: Sender<Notification>,
        connection_tx: oneshot::Sender<Result<(), ConnectError>>,
        request_rx: Receiver<Request>,
        command_rx: Receiver<Command>,
    ) {
        let mqtt_state = Arc::new(Mutex::new(MqttState::new(mqttoptions.clone())));
        let mut connection = Self {
            mqtt_state,
            notification_tx,
            connection_tx: Some(connection_tx),
            connection_count: 0,
            mqttoptions,
            is_network_enabled: true,
        };

        connection.mqtt_eventloop(request_rx, command_rx).await
    }

    /// Main mqtt event loop. Handles reconnection requests from `connect_or_not` and `mqtt_io`
    async fn mqtt_eventloop(&mut self, request_rx: Receiver<Request>, command_rx: Receiver<Command>) {
        let network_request_stream = request_rx;
        let mut network_request_stream = network_request_stream.prependable();
        let mut command_stream = Self::command_stream(command_rx);

        'reconnection: loop {
            let mqtt_connect_future = self.mqtt_connect().await;
            let framed = match self.connect_or_not(mqtt_connect_future).await {
                Ok(f) => f,
                Err(true) => continue 'reconnection,
                Err(false) => break 'reconnection,
            };

            // Insert previous session. If this is the first connect, the buffer in
            // network_request_stream is empty.
            network_request_stream.insert(self.mqtt_state.lock().await.handle_reconnection());

            let mqtt_future = self
                .mqtt_future(&mut command_stream, &mut network_request_stream, framed)
                .await;

            match self.mqtt_io(mqtt_future).await {
                Err(true) => continue 'reconnection,
                Err(false) => break 'reconnection,
                Ok(_v) => continue 'reconnection,
            }
        }
    }

    /// Makes a blocking mqtt connection an returns framed and reactor which created
    /// the connection when `is_network_enabled` flag is set true
    async fn connect_or_not(
        &mut self,
        mqtt_connect_future: impl Future<Output = Result<MqttFramed, ConnectError>>,
    ) -> Result<Option<MqttFramed>, bool> {
        let mqtt_connect_deadline = time::timeout(self.mqttoptions.connection_timeout(), mqtt_connect_future);

        if !self.is_network_enabled {
            return Ok(None);
        }

        async fn handle_connection_error(this: &mut Connection, error: ConnectError) -> bool {
            error!("Connection error = {:?}", error);

            // send connection error notification only the first time
            if let Some(connection_tx) = this.connection_tx.take() {
                connection_tx.send(Err(error)).unwrap();
            }

            this.should_reconnect_again().await
        };

        let framed = match mqtt_connect_deadline.await {
            Ok(Ok(framed)) => {
                info!("Mqtt connection successful!!");
                self.handle_connection_success();
                framed
            }
            Ok(Err(e)) => return Err(handle_connection_error(self, e).await),
            Err(_e) => return Err(handle_connection_error(self, ConnectError::Timeout).await),
        };

        Ok(Some(framed))
    }

    /// Tells whether eventloop should try to reconnect or not based
    /// user reconnection configuration
    async fn should_reconnect_again(&self) -> bool {
        let reconnect_options = self.mqttoptions.reconnect_opts();

        match reconnect_options {
            ReconnectOptions::Always(time) => {
                let time = Duration::from_secs(time);
                time::delay_for(time).await;
                true
            }
            ReconnectOptions::AfterFirstSuccess(time) => {
                // should reconnect only if initial connection was successful
                let reconnect = self.connection_count > 0;
                if reconnect {
                    let time = Duration::from_secs(time);
                    time::delay_for(time).await;
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
    async fn mqtt_io(&mut self, mqtt_future: impl Future<Output = Result<(), NetworkError>>) -> Result<(), bool> {
        let o = mqtt_future.await;
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
                NetworkError::NetworkStreamClosed if self.mqtt_state.lock().await.is_disconnecting() => {
                    self.is_network_enabled = false;
                    Err(false)
                }
                NetworkError::NetworkStreamClosed => {
                    self.is_network_enabled = true;
                    Err(self.should_reconnect_again().await)
                }
                _ => {
                    self.is_network_enabled = true;
                    Err(self.should_reconnect_again().await)
                }
            };
        }

        if let Ok(_v) = o {
            debug!("Eventloop stopped without error");
            return Err(self.should_reconnect_again().await);
        }

        Ok(())
    }

    /// Applies throttling and inflight limiting based on user configuration and returns
    /// a statful mqtt event loop future to be run on the reactor. The returned future also
    /// conditionally enable/disables network functionality based on the current `framed` state
    async fn mqtt_future(
        &mut self,
        command_stream: impl Stream<Item = Result<Packet, NetworkError>> + Unpin,
        network_request_stream: impl Stream<Item = Request> + Send + Unpin,
        framed: Option<Framed<NetworkStream, MqttCodec>>,
    ) -> impl Future<Output = Result<(), NetworkError>> {
        // convert a request stream to request packet stream after filtering
        // unnecessary requests and apply inflight limiting and rate limiting
        // note: make sure that the order remains (inflight, rate, request handling)
        // or else inflight limiting might face off by one bugs like progressing after
        // receiving 2 acks insteam of 1 ack
        let network_request_stream = self.inflight_limited_request_stream(network_request_stream).await;
        let network_request_stream = self.throttled_network_stream(network_request_stream);
        let network_request_stream = self.user_requests(network_request_stream);
        let network_request_stream = network_request_stream.map_ok(move |packet| -> Packet { packet.into() });

        // check if the network is enabled and create a future
        let network = match framed {
            Some(f) => {
                let (network_sink, network_stream) = f.split();
                let network_sink = network_sink.sink_map_err(NetworkError::Io);
                let network_reply_stream = self.network_reply_stream(network_stream).await;
                Ok((network_reply_stream, network_sink, command_stream))
            }
            None => Err(command_stream),
        };

        async move {
            match network {
                Ok((network_reply_stream, network_sink, command_stream)) => {
                    // convert requests to packets
                    let network_reply_stream = network_reply_stream.map_ok(|r| -> Packet { r.into() });
                    let network_stream = stream::select(network_reply_stream, network_request_stream);
                    let stream = stream::select(command_stream, network_stream);
                    Ok(stream.forward(network_sink).await?)
                }
                Err(command_stream) => {
                    let dummy_sink = BlackHole;
                    let f = command_stream.forward(dummy_sink);
                    Ok(f.await?)
                }
            }
        }
    }

    /// Sends connection status on blocked connections status call in `run`
    fn handle_connection_success(&mut self) {
        // send connection success notification only the first time
        if let Some(connection_tx) = self.connection_tx.take() {
            connection_tx.send(Ok(())).unwrap();
        } else {
            let _ = self.notification_tx.try_send(Notification::Reconnection);
        }

        self.connection_count += 1;
    }

    /// Resolves dns with blocking API and composes a future which makes a new tcp
    /// or tls connection to the broker. Note that this doesn't actual connect to the
    /// broker
    fn tcp_connect_future(&self) -> impl Future<Output = Result<MqttFramed, ConnectError>> {
        let (host, port) = self.mqttoptions.broker_address();
        let proxy = self.mqttoptions.proxy();

        let builder = NetworkStream::builder();

        let builder = if let Some(ca) = self.mqttoptions.ca() {
            let mut builder = builder.add_certificate_authority(&ca);
            if let Some(alpn) = self.mqttoptions.alpn() {
                builder = builder.add_alpn_protocols(&alpn);
            }

            if let Some((cert, key)) = self.mqttoptions.client_auth() {
                builder = builder.add_client_auth(&cert, &key);
            }

            builder
        } else {
            builder
        };

        let builder = match proxy {
            Proxy::None => builder,
            Proxy::HttpConnect(proxy_host, proxy_port, key, expiry) => {
                let id = self.mqttoptions.client_id();
                builder.set_http_proxy(&id, &proxy_host, proxy_port, &key, expiry)
            }
        };

        async move { builder.connect(&host, port).await }
    }

    /// Composes a new future which is a combination of tcp connect + mqtt handshake
    async fn mqtt_connect(&self) -> impl Future<Output = Result<MqttFramed, ConnectError>> {
        let mqtt_state = self.mqtt_state.clone();
        let tcp_connect_future = self.tcp_connect_future();
        let connect_packet = self.mqtt_state.lock().await.handle_outgoing_connect().unwrap();

        async move {
            let mut framed = tcp_connect_future.await?;
            let packet = Packet::Connect(connect_packet);
            framed.send(packet).await.map_err(ConnectError::Io)?;
            let (response, framed) = framed.into_future().await;
            let response = response.transpose().map_err(|err| ConnectError::Io(err))?;
            info!("Mqtt connect response = {:?}", response);
            let mut mqtt_state = mqtt_state.lock().await;
            check_and_validate_connack(response, framed, &mut mqtt_state)
        }
    }

    /// Handles all incoming network packets (including sending notifications to user over a
    /// channel) and creates a stream of packets to send on network
    async fn network_reply_stream(
        &self,
        network_stream: impl Stream<Item = Result<Packet, io::Error>> + Unpin,
    ) -> impl Stream<Item = Result<Request, NetworkError>> {
        struct Elapsed(());

        #[derive(Debug)]
        #[must_use = "futures do nothing unless you `.await` or poll them"]
        struct Timeout<T> {
            value: T,
            duration: Duration,
            timeout: time::Timeout<futures::future::Pending<()>>,
        }

        impl<T> Timeout<T> {
            fn new_timeout_future(duration: Duration) -> time::Timeout<futures::future::Pending<()>> {
                time::timeout(duration, futures::future::pending())
            }
        }

        impl<T> Stream for Timeout<T>
        where
            T: Stream,
        {
            type Item = Result<T::Item, Elapsed>;

            fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
                // Safety: T might be !Unpin, but we never move neither `value`
                // nor `delay`.
                //
                // ... X_X
                unsafe {
                    // First, try polling the future
                    let v = self.as_mut().map_unchecked_mut(|me| &mut me.value).poll_next(cx);

                    if let Poll::Ready(v) = v {
                        if v.is_some() {
                            let Self { duration, timeout, .. } = self.as_mut().get_unchecked_mut();
                            *timeout = Self::new_timeout_future(*duration);
                        }
                        return Poll::Ready(v.map(Ok));
                    }

                    // Now check the timer
                    match futures::ready!(self.as_mut().map_unchecked_mut(|me| &mut me.timeout).poll(cx)) {
                        Ok(()) => unreachable!(),
                        Err(_e) => {
                            let Self { duration, timeout, .. } = self.as_mut().get_unchecked_mut();
                            *timeout = Self::new_timeout_future(*duration);
                            Poll::Ready(Some(Err(Elapsed(()))))
                        }
                    }
                }
            }
        }

        fn timeout_stream<S>(duration: Duration, stream: S) -> Timeout<S>
        where
            S: Stream,
        {
            Timeout {
                value: stream,
                duration,
                timeout: Timeout::<S>::new_timeout_future(duration),
            }
        }

        let keep_alive = self.mqttoptions.keep_alive();
        let mut notification_tx = self.notification_tx.clone();

        let network_stream = timeout_stream(keep_alive, network_stream)
            .then({
                // FIXME: There SURELY must be a better way to do this...
                let mqtt_state = self.mqtt_state.clone();
                move |res| {
                    let mqtt_state = mqtt_state.clone();
                    async move {
                        match res {
                            Ok(m) => Ok(m?),
                            Err(_e) => {
                                debug!("Idle network incoming timeout");
                                mqtt_state
                                    .clone()
                                    .lock()
                                    .await
                                    .handle_outgoing_ping()
                                    .map(|_| Packet::Pingreq)
                            }
                        }
                    }
                }
            })
            .then({
                // FIXME: There SURELY must be a better way to do this...
                let mqtt_state = self.mqtt_state.clone();
                move |res| {
                    let mqtt_state = mqtt_state.clone();
                    async move {
                        match res {
                            Ok(packet) => {
                                debug!("Incoming packet = {:?}", packet_info(&packet));
                                mqtt_state.lock().await.handle_incoming_mqtt_packet(packet)
                            }
                            Err(e) => Err(e),
                        }
                    }
                }
            })
            .map(move |res| {
                res.and_then(|(notification, reply)| handle_notification_and_reply(&mut notification_tx, notification, reply))
            })
            .try_filter(|reply| {
                let should_forward = should_forward_packet(reply);
                async move { should_forward }
            });

        let network_reply_stream = network_stream.chain(stream::once(async { Err(NetworkError::NetworkStreamClosed) }));

        // when there are no outgoing replies, timeout should check if a ping is
        // necessary. E.g If there are only qos0 incoming publishes,
        // incoming network timeout will never trigger the ping. But broker needs a
        // ping when there are no outgoing packets. This timeout will take care of that
        // When network is completely idle, incoming network idle ping triggers first
        // and this timeout doesn't happen
        // When there are only qos0 incoming publishes, this timeout alone triggers
        let timeout = keep_alive + Duration::from_millis(500);
        timeout_stream(timeout, network_reply_stream)
            .then({
                let mqtt_state = self.mqtt_state.clone();
                move |res| {
                    let mqtt_state = mqtt_state.clone();
                    async move {
                        match res {
                            Ok(m) => Ok(m?),
                            Err(_e) => {
                                debug!("Idle network reply timeout");
                                let mut mqtt_state = mqtt_state.lock().await;
                                Ok(if mqtt_state.handle_outgoing_ping()? {
                                    Request::OutgoingIdlePing
                                } else {
                                    Request::None
                                })
                            }
                        }
                    }
                }
            })
            .try_filter(|reply| {
                let should_forward = should_forward_packet(reply);
                async move { should_forward }
            })
    }

    /// Handles all incoming user and session requests and creates a stream of packets to send
    /// on network
    /// All the remaining packets in the last session (when cleansession = false) will be prepended
    /// to user request stream to ensure that they are handled first. This cleanly handles last
    /// session stray (even if disconnect happens while sending last session data)because we always
    /// get back this stream from reactor after disconnection.
    fn user_requests(&mut self, request: impl Stream<Item = Request>) -> impl Stream<Item = Result<Request, NetworkError>> {
        // process user requests and convert them to network packets
        let request_stream = request.then({
            let mqtt_state = self.mqtt_state.clone();
            move |userrequest| {
                let mqtt_state = mqtt_state.clone();
                async move {
                    let mut mqtt_state = mqtt_state.lock().await;
                    validate_userrequest(userrequest, &mut mqtt_state)
                }
            }
        });

        request_stream.then({
            let mqtt_state = self.mqtt_state.clone();
            move |res| {
                let mqtt_state = mqtt_state.clone();
                async move {
                    match res {
                        Ok(packet) => {
                            let mut mqtt_state = mqtt_state.lock().await;
                            mqtt_state.handle_outgoing_mqtt_packet(packet)
                        }
                        Err(e) => Err(e),
                    }
                }
            }
        })
    }

    // Apply outgoing queue limit (in flights) by answering stream poll with not ready if queue is full
    // by returning NotReady.
    async fn inflight_limited_request_stream(
        &self,
        requests: impl Stream<Item = Request> + Send + Unpin,
    ) -> impl Stream<Item = Request> {
        let mqtt_state = self.mqtt_state.clone();
        let in_flight = self.mqttoptions.inflight();
        let mut stream = requests.peekable();

        rental! {
            mod rentals {
                use super::*;

                /// Stores a `Mutex` and a `MutexGuard` in the same struct.
                #[rental(debug, clone, covariant, map_suffix = "T")]
                pub struct RentMutex<H: 'static + stable_deref_trait::StableDeref + std::ops::Deref, T: 'static> {
                        head: H,
                        suffix: futures::lock::MutexLockFuture<'head, T>,
                }
            }
        }

        macro_rules! new_mutex_guard {
            ($mqtt_state: expr) => {
                rentals::RentMutex::new($mqtt_state, |s| s.lock())
            };
        }
        let mut mqtt_state = Some(new_mutex_guard!(mqtt_state));
        self::stream::poll_fn(move |cx| {
            let val = rentals::RentMutex::rent_mut(mqtt_state.as_mut().unwrap(), |s| match Pin::new(s).poll(cx) {
                Poll::Ready(mqtt_state) => {
                    let current_queue_len = (&*mqtt_state).publish_queue_len();
                    if current_queue_len < in_flight {
                        let poll = Pin::new(&mut stream).poll_next(cx);
                        poll
                    } else {
                        Poll::Pending
                    }
                }
                Poll::Pending => Poll::Pending,
            });
            {
                // XXX: There MUST be a better way to replace this with a new value...
                let head = rentals::RentMutex::into_head(mqtt_state.take().unwrap());
                mqtt_state.replace(new_mutex_guard!(head));
            }
            val
        })
    }

    /// Apply throttling if configured
    fn throttled_network_stream(&mut self, requests: impl Stream<Item = Request>) -> impl Stream<Item = Request> {
        if let Some(rate) = self.mqttoptions.throttle() {
            let duration = Duration::from_nanos((1_000_000_000.0 / rate) as u64);
            let throttled = time::throttle(duration, requests);
            Either::Left(throttled)
        } else {
            Either::Right(requests)
        }
    }

    /// Convert commands to errors
    fn command_stream(commands: mpsc::Receiver<Command>) -> impl Stream<Item = Result<Packet, NetworkError>> {
        // process user commands and raise appropriate error to the event loop
        commands.map(|usercommand| match usercommand {
            Command::Pause => Err(NetworkError::UserDisconnect),
            Command::Resume => Err(NetworkError::UserReconnect),
        })
    }
}

fn handle_notification_and_reply(
    notification_tx: &mut Sender<Notification>,
    notification: Notification,
    reply: Request,
) -> Result<Request, NetworkError> {
    match notification {
        Notification::None => Ok(reply),
        _ => match notification_tx.try_send(notification) {
            Ok(()) => Ok(reply),
            Err(e) => {
                error!("Notification send failed. Error = {:?}", e);
                Err(NetworkError::ReceiverCatchup)
            }
        },
    }
}

fn validate_userrequest(userrequest: Request, mqtt_state: &mut MqttState) -> Result<Packet, NetworkError> {
    match userrequest {
        Request::Reconnect(mqttoptions) => {
            mqtt_state.opts = mqttoptions;
            Err(NetworkError::UserReconnect)
        }
        _ => Ok(userrequest.into()),
    }
}

/// Checks if incoming packet is mqtt connack packet. Useful after mqtt
/// connect when we are waiting for connack but not any other packet.
fn check_and_validate_connack(
    packet: Option<Packet>,
    framed: MqttFramed,
    mqtt_state: &mut MqttState,
) -> Result<MqttFramed, ConnectError> {
    match packet {
        Some(Packet::Connack(connack)) => match mqtt_state.handle_incoming_connack(connack) {
            Err(err) => Err(err),
            _ => Ok(framed),
        },
        Some(packet) => Err(ConnectError::NotConnackPacket(packet)),
        None => Err(ConnectError::NoResponse),
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

#[derive(Debug)]
struct BlackHole;

impl Sink<Packet> for BlackHole {
    type Error = NetworkError;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, _item: Packet) -> Result<(), Self::Error> {
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod test {
    use super::{ConnectError, Connection, MqttFramed, MqttOptions, MqttState, NetworkError, ReconnectOptions};
    use crate::client::Notification;
    #[cfg(target_os = "linux")]
    use crate::client::Request;
    use futures::{
        channel::{mpsc, oneshot},
        future::{self, ready},
        lock::Mutex,
        stream::{self, Stream, StreamExt, TryStreamExt},
    };
    use mqtt311::{Packet, PacketIdentifier, Publish, QoS};
    #[cfg(target_os = "linux")]
    use std::time::Instant;
    use std::{io, sync::Arc, time::Duration};
    use tokio::time::DelayQueue;

    struct UserHandle {
        notification_rx: mpsc::Receiver<Notification>,
        connection_rx: oneshot::Receiver<Result<(), ConnectError>>,
    }

    fn mock_mqtt_connection(mqttoptions: MqttOptions, mqtt_state: MqttState) -> (Connection, UserHandle) {
        let (connection_tx, connection_rx) = oneshot::channel();
        let (notification_tx, notification_rx) = mpsc::channel(10);

        let mqtt_state = Arc::new(Mutex::new(mqtt_state));
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
            connection_rx,
        };

        (connection, userhandle)
    }

    #[cfg(target_os = "linux")]
    fn user_requests(delay: Duration) -> impl Stream<Item = Request> {
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

        requests.map(|v| v.unwrap().into_inner())
    }

    fn network_incoming_publishes(delay: Duration, count: u32) -> impl Stream<Item = Result<Packet, io::Error>> {
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

        publishes
            .map_ok(|v| v.into_inner())
            .map_err(|_e| io::Error::new(io::ErrorKind::Other, "Timer error"))
    }

    #[cfg(target_os = "linux")]
    fn network_incoming_acks(delay: Duration) -> impl Stream<Item = Result<Packet, io::Error>> {
        let mut acks = DelayQueue::new();

        for i in 1..=100 {
            acks.insert(Packet::Puback(PacketIdentifier(i as u16)), i * delay);
        }

        acks.map_ok(|v| v.into_inner())
            .map_err(|_e| io::Error::new(io::ErrorKind::Other, "Timer error"))
    }

    #[tokio::test]
    async fn run_should_raise_connection_errors_based_on_reconnection_options() {
        // local broker isn't running. Should result in connection errors
        let reconnect_opt = ReconnectOptions::Never;
        let mqttoptions = MqttOptions::new("mqtt-io-test", "localhost", 1883).set_reconnect_opts(reconnect_opt);

        // error in `never connect` case
        let o = Connection::run(mqttoptions).await;
        assert!(o.is_err());

        let reconnect_opt = ReconnectOptions::AfterFirstSuccess(10);
        let mqttoptions = MqttOptions::new("mqtt-io-test", "localhost", 1883).set_reconnect_opts(reconnect_opt);

        // error in `after first success` case
        let o = Connection::run(mqttoptions).await;
        assert!(o.is_err());

        // no error in `always` case
        let reconnect_opt = ReconnectOptions::Always(10);
        let mqttoptions = MqttOptions::new("mqtt-io-test", "localhost", 1883).set_reconnect_opts(reconnect_opt);

        let o = Connection::run(mqttoptions).await;
        assert!(o.is_ok());
    }

    #[tokio::test]
    async fn connect_or_not_returns_correct_reconnection_behaviour_in_always_reconnect_mode() {
        let reconnect_opt = ReconnectOptions::Always(10);
        let mqttoptions = MqttOptions::new("mqtt-io-test", "localhost", 1883).set_reconnect_opts(reconnect_opt);
        let mqtt_state = MqttState::new(mqttoptions.clone());

        // disconnections should take user reconnection options into consideration
        let (mut connection, userhandle) = mock_mqtt_connection(mqttoptions.clone(), mqtt_state);
        let ioerror = io::Error::new(io::ErrorKind::Other, "oh no!");
        let connect_future = future::err::<MqttFramed, _>(ConnectError::Io(ioerror));

        // results in an error but continues reconnection
        match connection.connect_or_not(connect_future).await {
            Err(true) => (),
            Err(false) => panic!("Should return reconnect = true"),
            Ok(_) => panic!("not possible"),
        }
        assert!(userhandle
            .connection_rx
            .await
            .expect("expected connection not to be cancelled")
            .is_err(),);
    }

    #[tokio::test]
    async fn connect_or_not_returns_dontreconnect_in_afterfirstsuccess_mode_during_first_failure() {
        // first connection
        let reconnect_opt = ReconnectOptions::AfterFirstSuccess(5);
        let mqttoptions = MqttOptions::new("mqtt-io-test", "localhost", 1883).set_reconnect_opts(reconnect_opt);
        let mqtt_state = MqttState::new(mqttoptions.clone());

        // disconnections should take user reconnection options into consideration
        let (mut connection, userhandle) = mock_mqtt_connection(mqttoptions.clone(), mqtt_state);
        let ioerror = io::Error::new(io::ErrorKind::Other, "oh no!");
        let connect_future = future::err::<MqttFramed, _>(ConnectError::Io(ioerror));

        // results in an error and reconnection = false during 1st reconnection
        match connection.connect_or_not(connect_future).await {
            Err(false) => (),
            Err(true) => panic!("Should return reconnect = false"),
            Ok(_) => panic!("not possible"),
        }
        assert!(userhandle
            .connection_rx
            .await
            .expect("expected connection not to be cancelled")
            .is_err(),);
    }

    #[tokio::test]
    async fn connect_or_not_returns_reconnect_in_afterfirstsuccess_mode_during_second_failure() {
        // first connection
        let reconnect_opt = ReconnectOptions::AfterFirstSuccess(3);
        let mqttoptions = MqttOptions::new("mqtt-io-test", "localhost", 1883).set_reconnect_opts(reconnect_opt);
        let mqtt_state = MqttState::new(mqttoptions.clone());

        // disconnections should take user reconnection options into consideration
        let (mut connection, _userhandle) = mock_mqtt_connection(mqttoptions.clone(), mqtt_state);
        connection.connection_count = 1;
        let ioerror = io::Error::new(io::ErrorKind::Other, "oh no!");
        let connect_future = future::err::<MqttFramed, _>(ConnectError::Io(ioerror));

        // results in an error and reconnection = false during 1st reconnection
        match connection.connect_or_not(connect_future).await {
            Err(true) => (),
            Err(false) => panic!("Should return reconnect = true"),
            Ok(_) => panic!("not possible"),
        }
    }

    #[tokio::test]
    async fn mqtt_io_returns_correct_reconnection_behaviour() {
        let reconnect_opt = ReconnectOptions::Always(10);
        let mqttoptions = MqttOptions::new("mqtt-io-test", "localhost", 1883).set_reconnect_opts(reconnect_opt);
        let mqtt_state = MqttState::new(mqttoptions.clone());

        // disconnections should take user reconnection options into consideration
        let (mut connection, _userhandle) = mock_mqtt_connection(mqttoptions.clone(), mqtt_state);
        let network_future = future::err::<(), _>(NetworkError::NetworkStreamClosed);
        let out = connection.mqtt_io(network_future).await;
        assert_eq!(out, Err(true));

        let mqtt_state = MqttState::new(mqttoptions.clone());
        // user shutdown should not take reconnection options into consideration
        let (mut connection, _userhandle) = mock_mqtt_connection(mqttoptions, mqtt_state);
        connection.mqtt_state.lock().await.handle_outgoing_disconnect().unwrap();
        let network_future = future::err::<(), _>(NetworkError::NetworkStreamClosed);
        let out = connection.mqtt_io(network_future).await;
        assert_eq!(out, Err(false));
    }

    #[cfg(target_os = "linux")]
    // incoming puback at second 1 and pingresp at periodic intervals
    fn network_incoming_pingresps() -> impl Stream<Item = Result<Packet, io::Error>> {
        let mut acks = DelayQueue::new();

        let publish = Publish {
            dup: false,
            qos: QoS::AtMostOnce,
            retain: false,
            pkid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        acks.insert(Packet::Publish(publish), Duration::from_secs(2));
        // out idle ping at 5000 + 500 (out ping delay wrt to keep alive)
        acks.insert(Packet::Pingresp, Duration::from_millis(5510));
        // in idle ping at 10520
        acks.insert(Packet::Pingresp, Duration::from_millis(10520));
        // in idle ping at 15530
        acks.insert(Packet::Pingresp, Duration::from_millis(15530));

        acks.map_ok(|v| v.into_inner())
            .map_err(|_e| io::Error::new(io::ErrorKind::Other, "Timer error"))
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn reply_stream_triggers_pings_on_time() {
        let mqttoptions = MqttOptions::default().set_keep_alive(5);
        let mqtt_state = MqttState::new(mqttoptions.clone());

        let (connection, _userhandle) = mock_mqtt_connection(mqttoptions, mqtt_state);
        let network_reply_stream = network_incoming_pingresps();
        let network_reply_stream = connection.network_reply_stream(network_reply_stream).await;

        let start = Instant::now();

        // incoming publish at second 1,
        // so pingreq should happen at second 6, 12 and 16
        let network_future = network_reply_stream.try_fold(1, |count, packet| {
            let elapsed = start.elapsed().as_millis();
            println!("Packet = {:?}, Elapsed = {:?}", packet, elapsed);
            match packet {
                // incoming publish at 2000. (in idle, out idle) = (7000, 5000 + 500) ---> out idle ping at 5500
                Request::OutgoingIdlePing if count == 1 => assert!(elapsed > 5500 && elapsed < 5700),
                // ping resp at 5510. (in idle, out idle) = (10510, 5500 + 5500) ---> in idle ping at 10510
                Request::IncomingIdlePing if count == 2 => assert!(elapsed > 10510 && elapsed < 10700),
                // ping resp at 10520. (in idl, out idle) = (15520, 10510 + 5500) ---> in idle ping at 15520
                Request::IncomingIdlePing if count == 3 => assert!(elapsed > 15520 && elapsed < 15700),
                _ => panic!("Expecting publish or ping"),
            }

            ready(Ok(count + 1))
        });

        match network_future.await {
            Err(NetworkError::NetworkStreamClosed) | Ok(_) => (),
            Err(e) => panic!("Error = {:?}", e),
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn throttled_stream_operates_at_specified_rate() {
        let mqttoptions = MqttOptions::default().set_throttle(5.0);
        let mqtt_state = MqttState::new(mqttoptions.clone());

        let (mut connection, _userhandle) = mock_mqtt_connection(mqttoptions, mqtt_state);

        // note: maintain order similar to mqtt_future()
        // generates 100 user requests
        let user_request_stream = user_requests(Duration::from_millis(1));
        let user_request_stream = connection.throttled_network_stream(user_request_stream);
        let user_request_stream = connection.user_requests(user_request_stream);
        let user_request_stream = user_request_stream.and_then(move |packet| future::ok(packet.into()));

        user_request_stream
            .try_fold(Instant::now(), |last, v: Packet| {
                // println!("outgoing = {:?}", v);
                let now = Instant::now();

                if let Packet::Publish(Publish { pkid, .. }) = v {
                    if pkid.unwrap() > PacketIdentifier(1) {
                        let elapsed = (now - last).as_millis();
                        dbg!(elapsed);
                        assert!(elapsed > 190 && elapsed < 220)
                    }
                }

                ready(Ok(now))
            })
            .await
            .unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn requests_should_block_during_max_in_flight_messages() {
        let mqttoptions = MqttOptions::default().set_inflight(50);
        let mqtt_state = MqttState::new(mqttoptions.clone());

        let (mut connection, _userhandle) = mock_mqtt_connection(mqttoptions, mqtt_state);

        // note: maintain order similar to mqtt_future()
        // generates 100 user requests
        let user_request_stream = user_requests(Duration::from_millis(1));
        let user_request_stream = connection.inflight_limited_request_stream(user_request_stream).await;
        let user_request_stream = connection.user_requests(user_request_stream);
        let user_request_stream = user_request_stream.map_ok(|r| r.into());

        // generates 100 acks
        let network_reply_stream = network_incoming_acks(Duration::from_millis(200));
        let network_reply_stream = connection.network_reply_stream(network_reply_stream).await;
        let network_reply_stream = network_reply_stream.map_ok(|r| r.into());
        let network_stream = stream::select(network_reply_stream, user_request_stream);
        let network_stream = network_stream.try_fold(Instant::now(), |last, v| {
            // println!("outgoing = {:?}", v);
            let now = Instant::now();

            if let Packet::Publish(Publish { pkid, .. }) = v {
                if pkid.unwrap() > PacketIdentifier(51) {
                    let elapsed = (now - last).as_millis();
                    dbg!(elapsed);
                    assert!(elapsed > 190 && elapsed < 220)
                }
            }

            ready(Ok(now))
        });
        let _ = network_stream.await;
    }

    #[tokio::test]
    async fn reply_stream_results_in_an_error_when_notification_receiver_doesnt_catchup() {
        let mqttoptions = MqttOptions::default().set_inflight(50);
        let mqtt_state = MqttState::new(mqttoptions.clone());

        let (connection, _userhandle) = mock_mqtt_connection(mqttoptions, mqtt_state);
        let network_reply_stream = network_incoming_publishes(Duration::from_millis(100), 12);
        let network_reply_stream = connection.network_reply_stream(network_reply_stream).await;

        let network_future = network_reply_stream.try_for_each(|_v| async move {
            eprintln!("Incoming = {:?}", _v);
            Ok(())
        });

        match network_future.await {
            Err(NetworkError::ReceiverCatchup) => (),
            other => panic!("Should result in receiver catchup error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn connection_success_and_disconnections_should_put_state_change_events_on_notifications() {
        let mqttoptions = MqttOptions::default().set_inflight(50);
        let mqtt_state = MqttState::new(mqttoptions.clone());

        let (mut connection, mut userhandle) = mock_mqtt_connection(mqttoptions, mqtt_state);
        connection.handle_connection_success();

        let thread = tokio::spawn(async move {
            let mut count = 0;
            while let Some(notification) = userhandle.notification_rx.next().await {
                match notification {
                    Notification::Reconnection if count == 0 => (),
                    Notification::Disconnection if count == 21 => (),
                    Notification::Publish(_) if count != 0 || count != 21 => (),
                    n => panic!("Not expected notification {:?}", n),
                }
                count += 1;
            }
        });

        // puts connection success event on the notifaction channel
        connection.handle_connection_success();
        let network_reply_stream = network_incoming_publishes(Duration::from_millis(100), 20);
        // end of the stream will simulate server disconnection
        let network_reply_stream = connection.network_reply_stream(network_reply_stream).await;
        let network_future = network_reply_stream.try_for_each(|_v| async move { Ok(()) });

        let _ = connection.mqtt_io(network_future).await;
        drop(connection);
        thread.await.unwrap();
    }
}

// fn print_last_session_state(
//     prepend: &mut Prepend<impl Stream<Item = Result<Request, NetworkError>>>,
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
