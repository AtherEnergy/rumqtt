use std::time::{Duration, Instant};
use std::net::{SocketAddr, ToSocketAddrs, Shutdown};
use std::collections::VecDeque;
use std::io::Write;
use std::str;
use std::net::TcpStream;
use std::sync::Arc;
use std::thread;
use std::sync::mpsc;

use mio::*;
use mio::timer::Timer;
use mio::channel::Receiver;
use mqtt::{Decodable, QualityOfService, TopicFilter};
use mqtt::packet::*;
use mqtt::control::variable_header::{ConnectReturnCode, PacketIdentifier};
use tls::{NetworkStream, SslContext};
use threadpool::ThreadPool;
use time;

use error::{Error, Result};
use message::Message;
use clientoptions::MqttOptions;
use request::{StatsReq, StatsResp, MqRequest};
use genpack;

const PING_TIMER: Token = Token(0);
const RETRANSMIT_TIMER: Token = Token(1);
const PUB0_CHANNEL: Token = Token(2);
const PUB1_CHANNEL: Token = Token(3);
const PUB2_CHANNEL: Token = Token(4);
const SUB_CHANNEL: Token = Token(5);
const INCOMING_CHANNEL: Token = Token(6);
const MISC_CHANNEL: Token = Token(7);
const STATS_CHANNEL: Token = Token(8);
// static mut N: i32 = 0;
// unsafe {
//     N += 1;
//     println!("N: {}", N);
// }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttState {
    Handshake,
    Connected,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttStatus {
    Success,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MioNotification {
    Disconnect,
    Shutdown,
    Reconnect,
}

enum HandlePacket {
    ConnAck,
    Publish(Box<Message>),
    PubAck(Option<Message>),
    PubRec(u16),
    // PubRel,
    PubComp,
    SubAck,
    UnSubAck,
    PingResp,
    Disconnect,
    Invalid,
}

pub type MessageSendableFn = Box<Fn(Message) + Send + Sync>;
pub type PublishSendableFn = Box<Fn(Message) + Send + Sync>;

/// Handles commands from Publisher and Subscriber. Saves MQTT
/// state and takes care of retransmissions.
pub struct MqttClient {
    pub addr: SocketAddr,
    pub opts: MqttOptions,
    pub stream: NetworkStream,

    // state
    pub state: MqttState,
    pub last_flush: Instant,
    pub last_pkid: PacketIdentifier,
    pub await_ping: bool,
    pub initial_connect: bool,
    // no. of pending messages in qos0 pub channel
    pub pub0_channel_pending: u32,
    // no. of pending messages in qos1 pub channel
    pub pub1_channel_pending: u32,
    // no. of pending messages in qos2 pub channel
    pub pub2_channel_pending: u32,
    // stop reading from all the publish channels when disconnected
    pub disconnect_block: bool,
    // stop reading from pub1 channel when qos1 queue is full(didn't get acks)
    pub should_qos1_block: bool,
    // stop reading from pub2 channel when qos2 queue is full
    pub should_qos2_block: bool,
    // no. of successful reconnections
    pub no_of_reconnections: u32,

    // Channels
    pub pub0_rx: Option<Receiver<Message>>,
    pub pub1_rx: Option<Receiver<Message>>,
    pub pub2_rx: Option<Receiver<Message>>,
    pub sub_rx: Option<Receiver<Vec<(TopicFilter, QualityOfService)>>>,
    pub incoming_rx: Option<Receiver<VariablePacket>>,
    pub misc_rx: Option<Receiver<MioNotification>>,
    pub stats_req_rx: Option<Receiver<StatsReq>>,
    pub stats_resp_tx: Option<mpsc::SyncSender<StatsResp>>,
    pub connsync_tx: Option<mpsc::SyncSender<MqttStatus>>,
    pub streamupdate_tx: Option<mpsc::SyncSender<NetworkStream>>,

    // Timers
    pub ping_timer: Option<Timer<String>>,
    pub retransmit_timer: Option<Timer<String>>,

    /// Queues. Note: 'record' is qos2 term for 'publish'
    /// For QoS 1. Stores outgoing publishes
    pub outgoing_pub: VecDeque<(i64, Box<Message>)>,
    /// For QoS 2. Store for incoming publishes to record.
    pub incoming_rec: VecDeque<Box<Message>>, //
    /// For QoS 2. Store for outgoing publishes.
    pub outgoing_rec: VecDeque<(i64, Box<Message>)>,
    /// For Qos2. Store for outgoing `pubrel` packets.
    pub outgoing_rel: VecDeque<(i64, PacketIdentifier)>,

    // clean_session=false will remember subscriptions only till lives.
    // If broker crashes, all its state will be lost (most brokers).
    // client wouldn't want to loose messages after it comes back up again
    pub subscriptions: VecDeque<Vec<(TopicFilter, QualityOfService)>>,

    /// On message callback
    pub message_callback: Option<Arc<MessageSendableFn>>,
    /// On publish callback
    pub publish_callback: Option<Arc<PublishSendableFn>>,
    pub pool: ThreadPool,

    /// Poll
    pub poll: Poll,
}

impl MqttClient {
    fn lookup_ipv4<A: ToSocketAddrs>(addr: A) -> SocketAddr {
        let addrs = addr.to_socket_addrs().expect("Conversion Failed");
        for addr in addrs {
            if let SocketAddr::V4(_) = addr {
                return addr;
            }
        }
        unreachable!("Cannot lookup address");
    }

    pub fn new(opts: MqttOptions) -> Self {
        let addr = opts.addr.clone();
        let addr = Self::lookup_ipv4(addr.as_str());
        // TODO: Move state initialization to MqttClient constructor
        MqttClient {
            addr: addr,
            stream: NetworkStream::None,

            // State
            last_flush: Instant::now(),
            last_pkid: PacketIdentifier(0),
            await_ping: false,
            state: MqttState::Disconnected,
            initial_connect: true,
            opts: opts,
            pub0_channel_pending: 0,
            pub1_channel_pending: 0,
            pub2_channel_pending: 0,
            disconnect_block: false,
            should_qos1_block: false,
            should_qos2_block: false,
            no_of_reconnections: 0,

            // Channels
            pub0_rx: None,
            pub1_rx: None,
            pub2_rx: None,
            sub_rx: None,
            incoming_rx: None,
            misc_rx: None,
            stats_req_rx: None,
            stats_resp_tx: None,
            connsync_tx: None,
            streamupdate_tx: None,

            // Timer
            ping_timer: None,
            retransmit_timer: None,

            // Queues
            incoming_rec: VecDeque::new(),
            outgoing_pub: VecDeque::new(),
            outgoing_rec: VecDeque::new(),
            outgoing_rel: VecDeque::new(),

            // Subscriptions
            subscriptions: VecDeque::new(),

            // Callbacks
            message_callback: None,
            publish_callback: None,

            // Threadpool
            pool: ThreadPool::new(3),

            // Poll
            poll: Poll::new().expect("Unable to create a poller"),
        }
    }

    // Note: Setting callback before subscriber & publisher
    // are created ensures that message callbacks are registered
    // before subscription & you don't need to pass callbacks through
    // channels (simplifies code)

    /// Set the message callback.  This is called when a message is
    /// received from the broker.
    pub fn message_callback<F>(mut self, callback: F) -> Self
        where F: Fn(Message) + Send + Sync + 'static
    {
        self.message_callback = Some(Arc::new(Box::new(callback)));
        self
    }

    /// Set the publish callback.  This is called when a published
    /// message has been sent to the broker successfully.
    pub fn publish_callback<F>(mut self, callback: F) -> Self
        where F: Fn(Message) + Send + Sync + 'static
    {
        self.publish_callback = Some(Arc::new(Box::new(callback)));
        self
    }

    /// Connects to the broker and starts an event loop in a new thread.
    /// Returns 'Request' and handles reqests from it.
    /// Also handles network events, reconnections and retransmissions.
    pub fn start(mut self) -> Result<MqRequest> {
        let (misc_tx, misc_rx) = channel::sync_channel::<MioNotification>(1);
        self.misc_rx = Some(misc_rx);

        let (stats_req_tx, stats_req_rx) = channel::sync_channel::<StatsReq>(1);
        self.stats_req_rx = Some(stats_req_rx);

        let (stats_resp_tx, stats_resp_rx) = mpsc::sync_channel::<StatsResp>(1);
        self.stats_resp_tx = Some(stats_resp_tx);

        let (pub0_tx, pub0_rx) = channel::sync_channel::<Message>(self.opts.pub_q_len as usize);
        self.pub0_rx = Some(pub0_rx);
        let (pub1_tx, pub1_rx) = channel::sync_channel::<Message>(self.opts.pub_q_len as usize);
        self.pub1_rx = Some(pub1_rx);
        let (pub2_tx, pub2_rx) = channel::sync_channel::<Message>(self.opts.pub_q_len as usize);
        self.pub2_rx = Some(pub2_rx);

        let (sub_tx, sub_rx) = channel::sync_channel::<Vec<(TopicFilter, QualityOfService)>>(self.opts.sub_q_len as usize);
        self.sub_rx = Some(sub_rx);

        let (incoming_tx, incoming_rx) = channel::sync_channel::<VariablePacket>(5);
        self.incoming_rx = Some(incoming_rx);

        // synchronizes tcp connection. why ?
        // start() call should fail if there a problem creating initial tcp
        // connection & mqtt connection. Since connections are happening inside thread,
        // this method should be informed to return error instead of
        // Request in case connection fails.
        let (connsync_tx, connsync_rx) = mpsc::sync_channel::<MqttStatus>(1);
        self.connsync_tx = Some(connsync_tx);

        let (streamupdate_tx, streamupdate_rx) = mpsc::sync_channel::<NetworkStream>(1);
        self.streamupdate_tx = Some(streamupdate_tx.clone());

        let ping_timer = Timer::default();
        self.ping_timer = Some(ping_timer);
        let retransmit_timer = Timer::default();
        self.retransmit_timer = Some(retransmit_timer);

        // @ Create Request through which user interacts
        // @ These are the handles using which user interacts with rumqtt.
        let mq_request = MqRequest {
            pub0_tx: pub0_tx,
            pub1_tx: pub1_tx,
            pub2_tx: pub2_tx,
            subscribe_tx: sub_tx,
            misc_tx: misc_tx.clone(),
            stats_req_tx: stats_req_tx,
            stats_resp_rx: stats_resp_rx,
        };

        // Initial Mqtt connection
        try!(self._try_reconnect());

        let reader_stream = self.stream.try_clone().expect("Couldn't clone the stream");
        try!(streamupdate_tx.send(reader_stream));

        // This is the thread that handles mio event loop.
        // Mio event loop is intentionally made to use just notify and timers.
        // Tcp Streams are std blocking streams.This helps avoiding the state
        // machine hell.
        // All the network writes also happen in this thread
        thread::spawn(move || -> Result<()> {
            // let poll = Poll::new().expect("Unable to create Poll");
            match self.run() {
                Ok(()) => println!("Event Loop returned with OK ???????"),
                Err(e) => {
                    error!("Stopping event loop. Error = {:?}", e);
                    return Err(e.into());
                }
            }
            Ok(())
        });

        // This thread handles network reads (coz they are blocking) and
        // and sends them to event loop thread to handle mqtt state.
        thread::spawn(move || -> Result<()> {
            'update_stream: loop {
                let mut stream = match streamupdate_rx.recv() {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Stream Update Error. error = {:?}", e);
                        return Err(e.into());
                    }
                };

                loop {
                    let packet = match VariablePacket::decode(&mut stream) {
                        // @ Decoded packet successfully.
                        Ok(pk) => pk,
                        Err(err) => {
                            // Socket error are readily available here as soon as
                            // disconnection happens. So it might be right for this
                            // thread to ask for reconnection rather than reconnecting
                            // during write failures
                            // UPDATE: Lot of publishes are being written by the time this notified
                            // the eventloop thread. Setting disconnect_block = true during write failure
                            error!("Error in receiving packet {:?}", err);
                            match misc_tx.send(MioNotification::Reconnect) {
                                Ok(_) => (),
                                Err(e) => {
                                    error!("Unable to send reconnect notification to eventloop thread. Error = {:?}", e);
                                    return Err(e.into());
                                }
                            }
                            continue 'update_stream;
                        }
                    };
                    match incoming_tx.send(packet) {
                        Ok(_) => (),
                        Err(e) => {
                            error!("Unable to send incoming packets to eventloop thread. Error = {:?}", e);
                            return Err(e.into());
                        }
                    }
                }
            }
        });

        let conn = try!(connsync_rx.recv());
        match conn {
            MqttStatus::Success => Ok(mq_request),
            MqttStatus::Failed => Err(Error::ConnectionAbort),
        }
    }

    fn run(mut self) -> Result<()> {
        let mut events = Events::new();
        {
            let ping_timer = self.ping_timer.as_ref().unwrap();
            self.poll.register(ping_timer, PING_TIMER, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
            let retransmit_timer = self.retransmit_timer.as_ref().unwrap();
            self.poll
                .register(retransmit_timer, RETRANSMIT_TIMER, Ready::readable(), PollOpt::edge())
                .expect("Poll Register Error");
            let pub0_rx = self.pub0_rx.as_ref().unwrap();
            self.poll.register(pub0_rx, PUB0_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
            let pub1_rx = self.pub1_rx.as_ref().unwrap();
            self.poll.register(pub1_rx, PUB1_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
            let pub2_rx = self.pub2_rx.as_ref().unwrap();
            self.poll.register(pub2_rx, PUB2_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
            let sub_rx = self.sub_rx.as_ref().unwrap();
            self.poll.register(sub_rx, SUB_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
            let incoming_rx = self.incoming_rx.as_ref().unwrap();
            self.poll.register(incoming_rx, INCOMING_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
            let misc_rx = self.misc_rx.as_ref().unwrap();
            self.poll.register(misc_rx, MISC_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
            let stats_req_rx = self.stats_req_rx.as_ref().unwrap();
            self.poll.register(stats_req_rx, STATS_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
        }

        loop {
            try!(self.poll.poll(&mut events, None));
            for e in events.iter() {
                match e.token() {
                    PUB0_CHANNEL => {
                        try!(self.publish0(false));
                    }
                    PUB1_CHANNEL => {
                        try!(self.publish1(false));
                    }
                    PUB2_CHANNEL => {
                        try!(self.publish2(false));
                    }
                    SUB_CHANNEL => {
                        try!(self.subscribe());
                    }
                    INCOMING_CHANNEL => {
                        try!(self.incoming());
                    }
                    PING_TIMER => {
                        try!(self.ping());
                    }
                    RETRANSMIT_TIMER => {
                        try!(self.retransmit());
                    }
                    MISC_CHANNEL => {
                        try!(self.misc());
                    }
                    STATS_CHANNEL => try!(self.stats()),
                    _ => panic!("Invalid Token"),
                };
            }
        }
    }

    fn ping(&mut self) -> Result<()> {
        debug!("client state --> {:?}, await_ping --> {}", self.state, self.await_ping);
        match self.state {
            MqttState::Connected => {
                if !self.await_ping {
                    let _ = self._ping();
                } else {
                    error!("awaiting for previous ping resp");
                }

                if let Some(keep_alive) = self.opts.keep_alive {
                    let ping_timer = self.ping_timer.as_mut().unwrap();
                    try!(ping_timer.set_timeout(Duration::from_millis(keep_alive as u64 * 900), "PING TIMER".to_string()));
                }
            }

            MqttState::Disconnected | MqttState::Handshake => error!("I won't ping. Client is in disconnected/handshake state"),
        }
        Ok(())
    }

    fn retransmit(&mut self) -> Result<()> {
        match self.state {
            MqttState::Connected => {
                debug!("^^^ QUEUE RESEND");
                self._try_retransmit();
            }
            MqttState::Disconnected | MqttState::Handshake => {
                debug!("I won't republish. Client is in disconnected/handshake state")
            }
        }
        let retransmit_timer = self.retransmit_timer.as_mut().unwrap();
        try!(retransmit_timer.set_timeout(Duration::from_millis(self.opts.queue_timeout as u64 * 1000),
                                                                            "PING TIMER".to_string()));
        Ok(())
    }

    fn subscribe(&mut self) -> Result<()> {
        let topics = {
            let sub_rx = self.sub_rx.as_ref().unwrap();
            try!(sub_rx.try_recv())
        };
        self.subscriptions.push_back(topics.clone());
        let _ = self._subscribe(topics);
        Ok(())
    }

    fn publish0(&mut self, clear_backlog: bool) -> Result<()> {
        // Increment only if notificication is from publisher
        if !clear_backlog {
            self.pub0_channel_pending += 1;
        }
        debug!("Block Reading = {}. Channel pending @@@@@ {}", self.disconnect_block, self.pub0_channel_pending);
        // Receive from publish qos0 channel only when connected.
        if !self.disconnect_block {
            loop {
                if self.pub0_channel_pending == 0 {
                    debug!("Finished everything in channel");
                    break;
                }
                let message = {
                    let pub0_rx = self.pub0_rx.as_ref().unwrap();
                    pub0_rx.try_recv().expect("Pub0 Rx Recv Error")
                };

                // NOTE: Reregister only after 'try_recv' is successful. Or else sync_channel
                // Buffer limit isn't working.
                {
                    let pub0_rx = self.pub0_rx.as_ref().unwrap();
                    self.poll.reregister(pub0_rx, PUB0_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
                }

                let _ = self._publish(message);
                self.pub0_channel_pending -= 1;
            }
        }
        Ok(())
    }

    fn publish1(&mut self, clear_backlog: bool) -> Result<()> {
        // Increment only if notificication is from publisher
        if !clear_backlog {
            self.pub1_channel_pending += 1;
        }

        debug!("## Clear Backlog = {}, QoS1 block = {}, Disconnect block = {}, Channel pending = {}",
               clear_backlog,
               self.should_qos1_block,
               self.disconnect_block,
               self.pub1_channel_pending);

        // Receive from publish qos1 channel only when outgoing pub queue
        // length is < max and in connected state
        if !self.should_qos1_block && !self.disconnect_block {
            loop {
                if self.pub1_channel_pending == 0 {
                    debug!("Finished everything in channel");
                    break;
                }
                let mut message = {
                    let pub1_rx = self.pub1_rx.as_ref().unwrap();
                    pub1_rx.try_recv().expect("Pub1 Rx Recv Error")
                };

                // NOTE: Reregister only after 'try_recv' is successful. Or else sync_channel
                // Buffer limit isn't working.
                {
                    let pub1_rx = self.pub1_rx.as_ref().unwrap();
                    self.poll.reregister(pub1_rx, PUB1_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
                }
                // Add next packet id to message and publish
                let PacketIdentifier(pkid) = self._next_pkid();
                message.set_pkid(pkid);
                let _ = self._publish(message);
                self.pub1_channel_pending -= 1;
                // println!("@@ Clear Backlog = {}, QoS1 block = {}, Disconnect block = {},
                // Channel pending = {}",
                //             clear_backlog,
                //             self.should_qos1_block,
                //             self.disconnect_block,
                //             self.pub1_channel_pending);
            }
        }
        Ok(())
    }

    fn publish2(&mut self, clear_backlog: bool) -> Result<()> {
        // Increment only if notificication is from publisher
        if !clear_backlog {
            self.pub2_channel_pending += 1;
        }

        debug!("QoS2 Channel pending @@@@@ {}", self.pub2_channel_pending);
        // Receive from publish qos2 channel only when outgoing pub queue
        // length is < max and in connected state
        if !self.should_qos2_block && !self.disconnect_block {
            loop {
                // Before
                if self.pub2_channel_pending == 0 {
                    debug!("Finished everything in channel");
                    break;
                }
                let mut message = {
                    let pub2_rx = self.pub2_rx.as_ref().unwrap();
                    pub2_rx.try_recv().expect("Pub2 Rx Recv Error")
                };

                // NOTE: Reregister only after 'try_recv' is successful. Or else sync_channel
                // Buffer limit isn't working.
                {
                    let pub2_rx = self.pub2_rx.as_ref().unwrap();
                    self.poll.reregister(pub2_rx, PUB2_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
                }

                // Add next packet id to message and publish
                let PacketIdentifier(pkid) = self._next_pkid();
                message.set_pkid(pkid);
                let _ = self._publish(message);
                self.pub2_channel_pending -= 1;
            }
        }
        Ok(())
    }

    fn incoming(&mut self) -> Result<()> {
        let packet = {
            let incoming_rx = self.incoming_rx.as_ref().unwrap();
            incoming_rx.try_recv().expect("Incoming Rx Recv Error")
        };

        // NOTE: Reregister only after 'try_recv' is successful. Or else sync_channel
        // Buffer limit isn't working.
        {
            let incoming_rx = self.incoming_rx.as_ref().unwrap();
            self.poll.reregister(incoming_rx, INCOMING_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
        }
        try!(self.STATE_handle_packet(&packet));
        Ok(())
    }

    fn misc(&mut self) -> Result<()> {
        let notification = {
            let misc_rx = self.misc_rx.as_ref().unwrap();
            try!(misc_rx.try_recv())
        };

        // NOTE: Reregister only after 'try_recv' is successful. Or else sync_channel
        // Buffer limit isn't working.
        {
            let misc_rx = self.misc_rx.as_ref().unwrap();
            self.poll.reregister(misc_rx, MISC_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
        }

        match notification {
            MioNotification::Disconnect => {
                debug!("{:?}", self.state);
                match self.state {
                    MqttState::Connected => {
                        let _ = self._disconnect();
                    }
                    _ => debug!("Mqtt connection not established"),
                }
            }
            MioNotification::Reconnect => {
                debug!("{:?}", self.state);

                self.state = MqttState::Disconnected;
                self.disconnect_block = true;
                loop {
                    match self._try_reconnect() {
                        Ok(_) => break,
                        Err(_) => continue,
                    }
                }

                // Handles the case where initial tcp connect is successful and mqtt connect
                // packets are sent (_try_reconnect) but broker closed the connection without
                // sending CONNACK. Broker might be expecting TLS or username & password
                if self.initial_connect {
                    let connsync_tx = self.connsync_tx.as_ref().unwrap();
                    connsync_tx.send(MqttStatus::Failed).expect("ConnSync Tx Send Error");
                } else {
                    let streamupdate_tx = self.streamupdate_tx.as_ref().unwrap();
                    let stream = self.stream.try_clone().expect("Stream Clone Error");
                    try!(streamupdate_tx.send(stream));
                }
            }
            MioNotification::Shutdown => {
                let _ = self.stream.shutdown(Shutdown::Both);
            }
        }
        Ok(())
    }

    fn stats(&self) -> Result<()> {
        let request = {
            let stats_req_rx = self.stats_req_rx.as_ref().unwrap();
            try!(stats_req_rx.try_recv())
        };

        // NOTE: Reregister only after 'try_recv' is successful. Or else sync_channel
        // Buffer limit isn't working.
        {
            let stats_req_rx = self.stats_req_rx.as_ref().unwrap();
            self.poll.reregister(stats_req_rx, STATS_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
        }
        let stats_resp_tx = self.stats_resp_tx.as_ref().unwrap();
        match request {
            StatsReq::QoS1QLen => try!(stats_resp_tx.send(StatsResp::QoS1QLen(self.outgoing_pub.len()))),
            StatsReq::QoS2QLen => try!(stats_resp_tx.send(StatsResp::QoS2QLen(self.outgoing_rec.len()))),
        }
        Ok(())
    }

    /// Return a count of (successful) mqtt connections that happened from the
    /// start.
    /// Just to know how many times the client reconnected (coz of bad
    /// networks, broker crashes etc)
    pub fn get_reconnection_count(&self) -> u32 { self.no_of_reconnections }

    #[allow(non_snake_case)]
    fn STATE_handle_packet(&mut self, packet: &VariablePacket) -> Result<()> {
        let handle = self.handle_packet(packet);
        if let Ok(p) = handle {
            match p {
                // Mqtt connection established, release (publisher, subscriber) & start the timers
                HandlePacket::ConnAck => {
                    self.state = MqttState::Connected;
                    self.no_of_reconnections += 1;
                    if self.initial_connect {
                        let connsync_tx = self.connsync_tx.as_ref().unwrap();
                        connsync_tx.send(MqttStatus::Success).expect("ConnSync Tx Send Error");
                        self.initial_connect = false;
                    } else {
                        // Resubscribe after a reconnection.
                        for s in self.subscriptions.clone() {
                            let _ = self._subscribe(s);
                        }
                        // Retransmit QoS1,2 queues after reconnection. Clears the queue by the time
                        // QoS*Reconnect notifications are sent to read pending messages in the channel
                        self._force_retransmit();
                        // Publisher won't stop even when disconnected until channel is full.
                        // This notifies notify() to publish channel pending messages after
                        // reconnect.
                        self.disconnect_block = false;
                        try!(self.publish0(true));
                        try!(self.publish1(true));
                        try!(self.publish2(true));
                    }

                    // TODO: Bug?? Multiple timers after restart? Doesn't seem so based on pings
                    let ping_timer = self.ping_timer.as_mut().unwrap();
                    let retransmit_timer = self.retransmit_timer.as_mut().unwrap();
                    try!(retransmit_timer.set_timeout(Duration::from_millis(self.opts.queue_timeout as u64 * 1000),
                                                                            "PING TIMER".to_string()));
                    if let Some(keep_alive) = self.opts.keep_alive {
                        try!(ping_timer.set_timeout(Duration::from_millis(keep_alive as u64 * 900), "PING TIMER".to_string()));
                    }

                }
                HandlePacket::Publish(m) => {
                    if let Some(ref message_callback) = self.message_callback {
                        let message_callback = message_callback.clone();
                        self.pool.execute(move || message_callback(*m));
                    }
                }
                HandlePacket::PubAck(m) => {
                    // Don't notify everytime q len is < max. This will always be true initially
                    // leading to dup notify.
                    // Send only for notify() to recover if channel is blocked.
                    // Blocking = true is set during publish if pub q len is more than desired.
                    if self.outgoing_pub.len() < self.opts.pub_q_len as usize && self.should_qos1_block {
                        self.should_qos1_block = false;
                        try!(self.publish1(true));
                    }

                    if m.is_none() {
                        return Ok(());
                    }

                    if let Some(ref publish_callback) = self.publish_callback {
                        let publish_callback = publish_callback.clone();
                        self.pool.execute(move || publish_callback(m.unwrap()));
                    }
                }
                // TODO: Better read from channel again after PubComp instead of PubRec
                HandlePacket::PubRec(pkid) => {
                    if self.outgoing_rec.len() < self.opts.pub_q_len as usize && self.should_qos2_block {
                        self.should_qos2_block = false;
                        try!(self.publish2(true));
                    }

                    // if let Some(ref publish_callback) = self.publish_callback {
                    //     let publish_callback = publish_callback.clone();

                    //     let pool = self.pool.as_mut().unwrap();
                    //     pool.execute(move || publish_callback(pkid));
                    // }
                }
                _ => debug!("packet handler says that he doesn't care"),
            }
        } else if let Err(err) = handle {
            error!("Error handling the packet {:?}", err);
        }
        Ok(())
    }

    fn handle_packet(&mut self, packet: &VariablePacket) -> Result<HandlePacket> {
        match self.state {
            MqttState::Handshake => {
                match *packet {
                    VariablePacket::ConnackPacket(ref connack) => {
                        let conn_ret_code = connack.connect_return_code();
                        if conn_ret_code != ConnectReturnCode::ConnectionAccepted {
                            error!("Failed to connect, err {:?}", conn_ret_code);
                            Err(Error::ConnectionRefused(conn_ret_code))
                        } else {
                            Ok(HandlePacket::ConnAck)
                        }
                    }
                    _ => {
                        error!("received invalid packet in handshake state --> {:?}", packet);
                        Ok(HandlePacket::Invalid)
                    }
                }
            }

            MqttState::Connected => {
                match *packet {
                    VariablePacket::SubackPacket(..) => {
                        // if ack.packet_identifier() != 10
                        // TODO: Maintain a subscribe queue and retry if
                        // subscribes are not successful
                        Ok(HandlePacket::SubAck)
                    }

                    VariablePacket::PingrespPacket(..) => {
                        self.await_ping = false;
                        Ok(HandlePacket::PingResp)
                    }

                    // @ Receives disconnect packet
                    VariablePacket::DisconnectPacket(..) => {
                        // TODO
                        Ok(HandlePacket::Disconnect)
                    }

                    // @ Receives puback packet and verifies it with sub packet id
                    VariablePacket::PubackPacket(ref puback) => {
                        // debug!("*** puback --> {:?}\n @@@ queue --> {:#?}",
                        //        puback,
                        //        self.outgoing_pub);
                        let pkid = puback.packet_identifier();
                        let m = match self.outgoing_pub
                            .iter()
                            .position(|ref x| x.1.get_pkid() == Some(pkid)) {
                            Some(i) => {
                                if let Some(m) = self.outgoing_pub.remove(i) {
                                    Some(*m.1)
                                } else {
                                    None
                                }
                            }
                            None => {
                                error!("Oopssss..unsolicited ack --> {:?}", puback);
                                None
                            }
                        };
                        debug!("Pub Q Len After Ack @@@ {:?}", self.outgoing_pub.len());
                        Ok(HandlePacket::PubAck(m))
                    }

                    // @ Receives publish packet
                    VariablePacket::PublishPacket(ref publ) => {
                        // unsafe {
                        //     N += 1;
                        //     println!("N: {}", N);
                        // }
                        let message = try!(Message::from_pub(publ));
                        self._handle_message(message)
                    }

                    // @ Qos2 message published by client is recorded by broker
                    // @ Remove message from 'outgoing_rec' queue and add pkid to 'outgoing_rel'
                    // @ Send 'pubrel' to broker
                    VariablePacket::PubrecPacket(ref pubrec) => {
                        let pkid = pubrec.packet_identifier();
                        match self.outgoing_rec
                            .iter()
                            .position(|ref x| x.1.get_pkid() == Some(pkid)) {
                            Some(i) => {
                                self.outgoing_rec.remove(i);
                            }
                            None => {
                                error!("Oopssss..unsolicited record --> {:?}", pubrec);
                            }
                        };

                        try!(self._pubrel(pkid));
                        self.outgoing_rel.push_back((time::get_time().sec, PacketIdentifier(pkid)));
                        Ok(HandlePacket::PubRec(pkid))
                    }

                    // @ Broker knows that client has the message
                    // @ release the message stored in 'recorded' queue
                    // @ send 'pubcomp' to sender indicating that message is released
                    // @ if 'pubcomp' packet is lost, broker will send pubrel again
                    // @ for the released message, for which we send dummy 'pubcomp' again
                    VariablePacket::PubrelPacket(ref pubrel) => {
                        let pkid = pubrel.packet_identifier();
                        let message = match self.incoming_rec
                            .iter()
                            .position(|ref x| x.get_pkid() == Some(pkid)) {
                            Some(i) => {
                                if let Some(message) = self.incoming_rec.remove(i) {
                                    Some(message)
                                } else {
                                    None
                                }
                            }
                            None => {
                                error!("Oopssss..unsolicited release. Message might have already \
                                        been released --> {:?}",
                                       pubrel);
                                None
                            }
                        };
                        try!(self._pubcomp(pkid));

                        if let Some(message) = message {
                            Ok(HandlePacket::Publish(message))
                        } else {
                            Ok(HandlePacket::Invalid)
                        }
                    }

                    // @ Remove this pkid from 'outgoing_rel' queue
                    VariablePacket::PubcompPacket(ref pubcomp) => {
                        let pkid = pubcomp.packet_identifier();
                        match self.outgoing_rel
                            .iter()
                            .position(|ref x| x.1 == PacketIdentifier(pkid)) {
                            Some(pos) => self.outgoing_rel.remove(pos),
                            None => {
                                error!("Oopssss..unsolicited complete --> {:?}", pubcomp);
                                None
                            }
                        };
                        Ok(HandlePacket::PubComp)
                    }

                    VariablePacket::UnsubackPacket(..) => Ok(HandlePacket::UnSubAck),

                    _ => Ok(HandlePacket::Invalid), //TODO: Replace this with panic later
                }
            }

            MqttState::Disconnected => {
                error!("Invalid (Disconnected) state while handling packets");
                Ok(HandlePacket::Invalid)
            }
        }
    }

    // TODO: Rename to handle incoming publish
    fn _handle_message(&mut self, message: Box<Message>) -> Result<HandlePacket> {
        debug!("       Publish {:?} {:?} < {:?} bytes",
               message.qos,
               message.topic.to_string(),
               message.payload.len());
        match message.qos {
            QoSWithPacketIdentifier::Level0 => Ok(HandlePacket::Publish(message)),
            QoSWithPacketIdentifier::Level1(pkid) => {
                try!(self._puback(pkid));
                Ok(HandlePacket::Publish(message))
            }

            // @ store the message in 'recorded' queue and send 'pubrec' to broker
            // @ if 'pubrec' is lost, broker will resend the message.
            // @ so only pushback is pkid is new. and resend pubcomp.
            // @ TODO: Analyze broker crash cases for all queues.
            QoSWithPacketIdentifier::Level2(pkid) => {
                match self.incoming_rec
                    .iter()
                    .position(|ref x| x.get_pkid() == Some(pkid)) {
                    Some(i) => {
                        self.incoming_rec[i] = message.clone();
                    }
                    None => {
                        self.incoming_rec.push_back(message.clone());
                    }
                };

                try!(self._pubrec(pkid));
                Ok(HandlePacket::PubRec(pkid))
            }
        }
    }

    fn _connect(&mut self) -> Result<()> {
        let connect = try!(genpack::generate_connect_packet(self.opts.client_id.clone(),
                                                            self.opts.clean_session,
                                                            self.opts.keep_alive,
                                                            self.opts.will.clone(),
                                                            self.opts.will_qos,
                                                            self.opts.will_retain,
                                                            self.opts.username.clone(),
                                                            self.opts.password.clone()));
        try!(self._write_packet(connect));
        self._flush()
    }

    pub fn _disconnect(&mut self) -> Result<()> {
        let disconnect = try!(genpack::generate_disconnect_packet());
        try!(self._write_packet(disconnect));
        self._flush()
    }


    fn _try_reconnect(&mut self) -> Result<()> {
        match self.opts.reconnect {
            // TODO: Implement
            None => panic!("To be implemented"),
            Some(dur) => {
                if !self.initial_connect {
                    error!("  Will try Reconnect in {} seconds", dur);
                    thread::sleep(Duration::new(dur as u64, 0));
                }
                let stream = try!(TcpStream::connect(&self.addr));
                let stream = match self.opts.ca {
                    Some(ref ca) => {
                        if let Some((ref crt, ref key)) = self.opts.client_cert {
                            let ssl_ctx: SslContext = try!(SslContext::new(ca, Some((crt, key))));
                            NetworkStream::Tls(try!(ssl_ctx.connect(stream)))
                        } else {
                            let ssl_ctx: SslContext = try!(SslContext::new(ca, None::<(String, String)>));
                            NetworkStream::Tls(try!(ssl_ctx.connect(stream)))
                        }
                    }
                    None => NetworkStream::Tcp(stream),
                };
                self.stream = stream;
                try!(self._connect());
                // TODO: Change states properly in one location
                self.state = MqttState::Handshake;
                Ok(())
            }
        }
    }

    fn _try_retransmit(&mut self) {
        match self.state {
            MqttState::Connected => {
                let timeout = self.opts.queue_timeout as i64;

                // Republish QoS 1 outgoing publishes
                while let Some(index) = self.outgoing_pub
                    .iter()
                    .position(|ref x| time::get_time().sec - x.0 > timeout) {
                    // println!("########## {:?}", self.outgoing_pub);
                    let message = self.outgoing_pub.remove(index).expect("No such entry");
                    let _ = self._publish(*message.1);
                }

                // Republish QoS 2 outgoing records
                while let Some(index) = self.outgoing_rec
                    .iter()
                    .position(|ref x| time::get_time().sec - x.0 > timeout) {
                    let message = self.outgoing_rec.remove(index).expect("No such entry");
                    let _ = self._publish(*message.1);
                }

                let outgoing_rel = self.outgoing_rel.clone(); //TODO: Remove the clone
                // Resend QoS 2 outgoing release
                for e in outgoing_rel.iter().filter(|ref x| time::get_time().sec - x.0 > timeout) {
                    let PacketIdentifier(pkid) = e.1;
                    let _ = self._pubrel(pkid);
                }
            }

            MqttState::Disconnected | MqttState::Handshake => error!("I won't republish. Client isn't in connected state"),
        }
    }

    // Spec says that client (for QoS > 0, clean session) should retransmit all the
    // unacked messages after reconnection. Instead of waiting for retransmit
    // timeout
    // to kickin, this methods retransmits everthing in the queue immediately
    // NOTE: outgoing_rels are handled in _try_retransmit. Sending duplicate pubrels
    // isn't a problem (I guess ?). Broker will just resend pubcomps
    fn _force_retransmit(&mut self) {
        if self.opts.clean_session {
            // We are anyway going to clear the queues if they aren't empty
            self.should_qos1_block = false;
            self.should_qos2_block = false;
            match self.state {
                MqttState::Connected => {
                    for index in 0..self.outgoing_pub.len() {
                        let message = self.outgoing_pub.remove(index).expect("No such entry");
                        let _ = self._publish(*message.1);
                    }

                    for index in 0..self.outgoing_rec.len() {
                        let message = self.outgoing_rec.remove(index).expect("No such entry");
                        let _ = self._publish(*message.1);
                    }
                }
                MqttState::Disconnected | MqttState::Handshake => {
                    error!("I won't force republish. Client isn't in connected state")
                }
            }
        }
    }

    fn _ping(&mut self) -> Result<()> {
        let ping = try!(genpack::generate_pingreq_packet());
        self.await_ping = true;
        try!(self._write_packet(ping));
        self._flush()
    }

    fn _unbind(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
        self.await_ping = false;
        self.state = MqttState::Disconnected;
        info!("  Disconnected {:?}", self.opts.client_id);
    }

    fn _subscribe(&mut self, topics: Vec<(TopicFilter, QualityOfService)>) -> Result<()> {
        let subscribe_packet = try!(genpack::generate_subscribe_packet(topics));
        try!(self._write_packet(subscribe_packet));
        self._flush()
    }

    fn _publish(&mut self, message: Message) -> Result<()> {

        let qos = message.qos;
        let message_box = message.transform(Some(qos));
        let topic = message.topic;
        let payload = &*message.payload;
        let retain = message.retain;

        let publish_packet = try!(genpack::generate_publish_packet(topic, qos, retain, payload.clone()));

        match message.qos {
            QoSWithPacketIdentifier::Level0 => (),
            QoSWithPacketIdentifier::Level1(_) => {
                self.outgoing_pub.push_back((time::get_time().sec, message_box.clone()));
                if self.outgoing_pub.len() >= self.opts.pub_q_len as usize {
                    self.should_qos1_block = true;
                }
            }
            QoSWithPacketIdentifier::Level2(_) => {
                self.outgoing_rec.push_back((time::get_time().sec, message_box.clone()));
                if self.outgoing_rec.len() >= self.opts.pub_q_len as usize {
                    self.should_qos2_block = true;
                }
            }
        }
        // debug!("       Publish {:?} {:?} > {} bytes", message.qos,
        // topic.clone().to_string(), message.payload.len());

        // TODO: print error for failure here
        try!(self._write_packet(publish_packet));
        self._flush()
    }

    fn _puback(&mut self, pkid: u16) -> Result<()> {
        let puback_packet = try!(genpack::generate_puback_packet(pkid));
        try!(self._write_packet(puback_packet));
        self._flush()
    }

    fn _pubrec(&mut self, pkid: u16) -> Result<()> {
        let pubrec_packet = try!(genpack::generate_pubrec_packet(pkid));
        try!(self._write_packet(pubrec_packet));
        self._flush()
    }

    fn _pubrel(&mut self, pkid: u16) -> Result<()> {
        let pubrel_packet = try!(genpack::generate_pubrel_packet(pkid));
        try!(self._write_packet(pubrel_packet));
        self._flush()
    }

    fn _pubcomp(&mut self, pkid: u16) -> Result<()> {
        let puback_packet = try!(genpack::generate_pubcomp_packet(pkid));
        try!(self._write_packet(puback_packet));
        self._flush()
    }

    fn _flush(&mut self) -> Result<()> {
        try!(self.stream.flush());
        self.last_flush = Instant::now();
        Ok(())
    }

    #[inline]
    fn _write_packet(&mut self, packet: Vec<u8>) -> Result<()> {
        match self.stream.write_all(&packet) {
            Ok(v) => v,
            Err(e) => {
                error!("Error writing packet. Error = {:?}", e);
                // disconnect block in case of socket errors. verify
                // self.disconnect_block = true
                return Err(e.into());
            }
        };
        Ok(())
    }

    // http://stackoverflow.
    // com/questions/11115364/mqtt-messageid-practical-implementation
    #[inline]
    fn _next_pkid(&mut self) -> PacketIdentifier {
        let PacketIdentifier(pkid) = self.last_pkid;
        self.last_pkid = PacketIdentifier(pkid + 1);
        self.last_pkid
    }
}

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

#[cfg(test)]
mod test {
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use mqtt::packet::*;
    use mqtt::QualityOfService as QoS;
    use mqtt::topic_name::TopicName;
    use time;

    use clientoptions::MqttOptions;
    use super::MqttClient;
    use message::Message;

    fn fill_qos1_publish_buffer(client: &mut MqttClient) {
        for i in 1001..1101 {
            let message = Box::new(Message {
                topic: TopicName::new("a/b/c".to_string()).unwrap(),
                qos: QoSWithPacketIdentifier::Level1(i),
                retain: false,
                payload: Arc::new("dummy data".to_string().into_bytes()),
                userdata: None,
            });
            client.outgoing_pub.push_back((time::get_time().sec, message));
        }
    }

    fn fill_qos2_publish_buffer(client: &mut MqttClient) {
        for i in 1..101 {
            let message = Box::new(Message {
                topic: TopicName::new("a/b/c".to_string()).unwrap(),
                qos: QoSWithPacketIdentifier::Level2(i),
                retain: false,
                payload: Arc::new("dummy data".to_string().into_bytes()),
                userdata: None,
            });
            client.outgoing_rec.push_back((time::get_time().sec, message));
        }
    }

    #[test]
    fn retransmission_after_timeout() {
        let client_options = MqttOptions::new()
            .set_keep_alive(5)
            .set_q_timeout(5)
            .set_client_id("test-retransmission-client")
            .broker("broker.hivemq.com:1883");

        let mut mq_client = MqttClient::new(client_options);
        fill_qos1_publish_buffer(&mut mq_client);
        fill_qos2_publish_buffer(&mut mq_client);

        // Connects to a broker and returns a `Publisher` and `Subscriber`
        let request = mq_client.start().expect("Coudn't start");
        thread::sleep(Duration::new(20, 0));
        let final_qos1_length = request.qos1_q_len().expect("Stats Request Error");
        let final_qos2_length = request.qos2_q_len().expect("Stats Request Error");
        assert_eq!(0, final_qos1_length);
        assert_eq!(0, final_qos2_length);
    }

    #[test]
    fn channel_block_and_unblock_after_retransmit_timeout() {
        let client_options = MqttOptions::new()
            .set_keep_alive(5)
            .set_q_timeout(5)
            .set_client_id("test-block-retransmission-client")
            .broker("broker.hivemq.com:1883");

        let mut mq_client = MqttClient::new(client_options);
        fill_qos1_publish_buffer(&mut mq_client);
        fill_qos2_publish_buffer(&mut mq_client);

        // Connects to a broker and returns a `Publisher` and `Subscriber`
        let request = mq_client.start().expect("Coudn't start");
        for i in 0..100 {
            let payload = format!("{}. hello rust", i);
            request.publish("test/qos1/blockretransmit", QoS::Level1, payload.into_bytes()).unwrap();
        }

        for i in 0..100 {
            let payload = format!("{}. hello rust", i);
            request.publish("test/qos2/blockretransmit", QoS::Level2, payload.into_bytes()).unwrap();
        }
        thread::sleep(Duration::new(20, 0));
        let final_qos1_length = request.qos1_q_len().expect("Stats Request Error");
        let final_qos2_length = request.qos2_q_len().expect("Stats Request Error");
        assert_eq!(0, final_qos1_length);
        assert_eq!(0, final_qos2_length);
    }
}

// Why RuMqtt:
// GOALS
// -----
// 1. Synchronous mqtt connects: No need of callback to check if mqtt
// connection is
// successful or not. You'll know of of errors (if any) synchronously
// 2. Synchronous subscribes (TODO): Same as above
// 3. Queued publishes: publishes won't throw errors by default. A queue (with
// user defined
// length) will be buffered when the n/w is down. If n/w is down for some time
// and queue
// becomes full, publishes are blocked
// 4. No locks. Fast and efficient because of Rust and Mio
// 5. Callback only for subscibed incoming message. Callbacks are executed
// using threadpool
//
