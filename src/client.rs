use std::time::{Duration, Instant};
use time;

use std::net::{SocketAddr, Shutdown};
use std::collections::VecDeque;
use std::io::Write;
use std::str;
use mio::tcp::TcpStream;
use mio::*;
use mqtt::{Encodable, Decodable, QualityOfService, TopicFilter};
use mqtt::packet::*;
use mqtt::control::variable_header::{ConnectReturnCode, PacketIdentifier};
use mqtt::topic_name::TopicName;
use std::sync::Arc;
use std::thread;
use tls::NetworkStream;
use std::sync::mpsc;

use error::{Error, Result};
use message::Message;
use clientoptions::MqttOptions;
use publisher::Publisher;
use subscriber::{Subscriber, SendableFn};

const MIO_PING_TIMER: u64 = 123;
const MIO_QUEUE_TIMER: u64 = 321;
const MIO_CLIENT_STREAM: Token = Token(1);

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
pub enum PubNotify {
    QoS0,
    QoS1,
    QoS1QueueDown,
    QoS1Reconnect,
    QoS2Reconnect,
    QoS2,
    QoS2QueueDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MioNotification {
    Pub(PubNotify),
    Sub,
    Disconnect,
    Shutdown,
}

enum HandlePacket {
    ConnAck,
    Publish(Box<Message>),
    PubAck,
    PubRec,
    // PubRel,
    PubComp,
    SubAck,
    UnSubAck,
    PingResp,
    Disconnect,
    Invalid,
}


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
    pub pub1_channel_pending: u32,
    pub pub2_channel_pending: u32,
    pub should_qos1_block: bool,
    pub should_qos2_block: bool,
    pub no_of_reconnections: u32,

    // Channels
    pub pub0_rx: Option<mpsc::Receiver<Message>>,
    pub pub1_rx: Option<mpsc::Receiver<Message>>,
    pub pub2_rx: Option<mpsc::Receiver<Message>>,
    pub sub_rx: Option<mpsc::Receiver<Vec<(TopicFilter, QualityOfService)>>>,
    pub connsync_tx: Option<mpsc::SyncSender<MqttStatus>>,
    pub mionotify_tx: Option<Sender<MioNotification>>,

    /// Queues. Note: 'record' is qos2 term for 'publish'
    /// For QoS 1. Stores outgoing publishes
    pub outgoing_pub: VecDeque<(i64, Box<Message>)>,
    /// For QoS 2. Store for incoming publishes to record.
    pub incoming_rec: VecDeque<Box<Message>>, //
    /// For QoS 2. Store for outgoing publishes.
    pub outgoing_rec: VecDeque<(i64, Box<Message>)>,
    /// For Qos2. Store for outgoing `pubrel` packets.
    pub outgoing_rel: VecDeque<(i64, PacketIdentifier)>,

    /// On message callback
    pub callback: Option<Arc<SendableFn>>,
}

// TODO: Use Mio::Handler, Unit test for state machine
impl Handler for MqttClient {
    type Timeout = u64;
    type Message = MioNotification;

    // TODO: Understand --> when connecting to localbroker, For Incoming publishes,
    // state is at Readable@Connected state but while connecting to remote broker,
    // state is at Writable@Connected. Hence ended up add STATE_reate,
    // STATE_handle_packet
    // to both readable and writable
    fn ready(&mut self, event_loop: &mut EventLoop<MqttClient>, token: Token, events: EventSet) {
        if events.is_readable() {
            info!("@@@ Readable ... {:?}", self.state);
            match token {
                MIO_CLIENT_STREAM => {
                    match self.state {
                        MqttState::Connected | MqttState::Handshake => {
                            // @ Get incoming packets from the broker.
                            let packet = match self.STATE_read_incoming(event_loop) {
                                Ok(pk) => {
                                    event_loop.reregister(self.stream.get_ref().unwrap(),
                                                    MIO_CLIENT_STREAM,
                                                    EventSet::readable(),
                                                    PollOpt::edge() | PollOpt::oneshot())
                                        .unwrap();
                                    pk
                                }
                                // @ If failed with EOF error, server has closed the socket.
                                // @ Set state to disconnected, make a new TCP connection (we need to register here)
                                // @ and event loop to writable. Now writable state machine
                                // @ will takeover restarting the mqtt connection procedure
                                // TODO: Get proper error instead of Error::Read
                                Err(e) => {
                                    error!("Error reading incoming packets : {:?}", e);
                                    self.state = MqttState::Disconnected;
                                    self.STATE_try_reconnect(event_loop);
                                    let stream = self.stream.get_ref().expect("No Stream 22");
                                    // NOTE: reregister isn't working for new streams in linux while
                                    // it's working in mac osx
                                    event_loop.register(stream,
                                                    MIO_CLIENT_STREAM,
                                                    EventSet::writable(),
                                                    PollOpt::edge() | PollOpt::oneshot())
                                        .expect("Couldn't reregister");
                                    return;
                                }
                            };

                            trace!("{:?}", packet);
                            self.STATE_handle_packet(&packet, event_loop);

                        }
                        MqttState::Disconnected => {
                            error!("Disconnected state invalid in readable");
                            event_loop.reregister(self.stream.get_ref().unwrap(),
                                            MIO_CLIENT_STREAM,
                                            EventSet::writable(),
                                            PollOpt::edge() | PollOpt::oneshot())
                                .unwrap();
                        }
                    }
                }

                _ => panic!("Invalid token .."),
            }
        }

        // You do not need to register EventSet::writable() if you want to immediately
        // write back
        // to the socket you just read from. You can just perform the write as part of
        // the current
        // readable event. If you are performing an expensive task between the read and
        // write, you may want to handle this differently. Whether you are reading or
        // writing from
        // the socket, you also need to be aware that the kernel might not be ready for
        // your read
        // r write.
        //
        // NOTE: THIS IS BEING INVOKED AFTER TCPSTREAM::CONNECT (server up or not)
        // It might be READABLE for new incoming connections (for server) or data
        if events.is_writable() {
            info!("%%% Writable ... {:?}", self.state);
            match token {
                MIO_CLIENT_STREAM => {
                    match self.state {
                        MqttState::Disconnected => {
                            debug!("sending mqtt connect packet ..");
                            match self._connect() {
                                // @ If writing connect packet is successful means TCP
                                // @ connection is successful. Change the state machine
                                // @ to handshake and event loop to readable to read
                                // @ CONACK packet
                                Ok(_) => {
                                    self.state = MqttState::Handshake;
                                    event_loop.reregister(self.stream.get_ref().unwrap(),
                                                    MIO_CLIENT_STREAM,
                                                    EventSet::readable(),
                                                    PollOpt::edge() | PollOpt::oneshot())
                                        .unwrap();
                                }
                                // @ Writing connect packet failed. Tcp connection might
                                // @ have failed. Try to establish a new Tcp connection
                                // @ and set eventloop to writable to write connect packets
                                // @ again
                                Err(e) => {
                                    error!("Error establishing tcp connection --> {:?}", e);
                                    self.state = MqttState::Disconnected;
                                    self.STATE_try_reconnect(event_loop);
                                    // Should be done after reconnect coz stream is updated
                                    let stream = self.stream.get_ref().expect("No stream 1");
                                    event_loop.register(stream,
                                                    MIO_CLIENT_STREAM,
                                                    EventSet::writable(),
                                                    PollOpt::edge() | PollOpt::oneshot())
                                        .unwrap();
                                }

                            }
                        }
                        MqttState::Handshake | MqttState::Connected => {
                            error!("Incoming data in writable state ..");
                            // TODO: Check if this is an error and we can do reads here
                            // directly
                            event_loop.reregister(self.stream.get_ref().unwrap(),
                                            MIO_CLIENT_STREAM,
                                            EventSet::readable(),
                                            PollOpt::edge() | PollOpt::oneshot())
                                .unwrap();

                        }
                    }
                }

                _ => panic!("Invalid token .."),
            }
        }
    }

    fn timeout(&mut self, event_loop: &mut EventLoop<Self>, timer: Self::Timeout) {
        //TODO: Move timer handling logic to seperate methods
        match timer {
            MIO_PING_TIMER => {
                debug!("client state --> {:?}, await_ping --> {}", self.state, self.await_ping);

                match self.state {
                    MqttState::Connected => {
                        if !self.await_ping {
                            let _ = self.ping();
                        } else {
                            error!("awaiting for previous ping resp");
                        }

                        if let Some(keep_alive) = self.opts.keep_alive {
                            event_loop.timeout_ms(MIO_PING_TIMER, keep_alive as u64 * 900).unwrap();
                        }
                    }

                    MqttState::Disconnected | MqttState::Handshake => {
                        error!("I won't ping. Client is in disconnected/handshake state")
                    }
                }
            }

            MIO_QUEUE_TIMER => {
                match self.state {
                    MqttState::Connected => {
                        debug!("^^^ QUEUE RESEND");
                        self._try_retransmit();
                    }
                    MqttState::Disconnected | MqttState::Handshake => {
                        debug!("I won't republish. Client is in disconnected/handshake state")
                    }
                }
                event_loop.timeout_ms(MIO_QUEUE_TIMER, self.opts.queue_timeout as u64 * 1000)
                    .unwrap();

            }

            _ => panic!("Invalid timer id"),

        }
    }

    //TODO: Make smaller methods
    fn notify(&mut self, _: &mut EventLoop<Self>, notification_type: MioNotification) {
        match self.state {
            MqttState::Connected => {
                match notification_type {
                    MioNotification::Pub(p) => {
                        match p {
                            PubNotify::QoS0 => {
                                let message = {
                                    match self.pub0_rx {
                                        Some(ref pub0_rx) => pub0_rx.recv().unwrap(),
                                        None => panic!("No publish recv channel"),
                                    }
                                };
                                let _ = self._publish(message);
                            }
                            PubNotify::QoS1 |
                            PubNotify::QoS1QueueDown |
                            PubNotify::QoS1Reconnect => {
                                // Increment only if notificication is from publisher
                                if p == PubNotify::QoS1 {
                                    self.pub1_channel_pending += 1;
                                }

                                // Receive from publish channel only when outgoing pub queue
                                // length is < max
                                if self.should_qos1_block == false {
                                    loop {
                                        debug!("Channel pending @@@@@ {}", self.pub1_channel_pending);
                                        // Before
                                        if self.pub1_channel_pending == 0 {
                                            debug!("Finished everything in channel");
                                            break;
                                        }
                                        let mut message = {

                                            match self.pub1_rx {
                                                // Careful, this is a blocking call. Might
                                                // be easier to find queue len bugs with this.
                                                Some(ref pub1_rx) => pub1_rx.recv().unwrap(),
                                                None => panic!("No publish recv channel"),
                                            }
                                        };
                                        // Add next packet id to message and publish
                                        let PacketIdentifier(pkid) = self._next_pkid();
                                        message.set_pkid(pkid);
                                        let _ = self._publish(message);
                                        self.pub1_channel_pending -= 1;
                                    }
                                }
                            }

                            PubNotify::QoS2 |
                            PubNotify::QoS2QueueDown |
                            PubNotify::QoS2Reconnect => {
                                // Increment only if notificication is from publisher
                                if p == PubNotify::QoS2 {
                                    self.pub2_channel_pending += 1;
                                }

                                // Receive from publish channel only when outgoing pub queue
                                // length is < max
                                if self.should_qos2_block == false {
                                    loop {
                                        debug!("QoS2 Channel pending @@@@@ {}", self.pub2_channel_pending);
                                        // Before
                                        if self.pub2_channel_pending == 0 {
                                            debug!("Finished everything in channel");
                                            break;
                                        }
                                        let mut message = {

                                            match self.pub2_rx {
                                                // Careful, this is a blocking call. Might
                                                // be easier to find queue len bugs with this.
                                                Some(ref pub2_rx) => pub2_rx.recv().unwrap(),
                                                None => panic!("No publish recv channel"),
                                            }
                                        };
                                        // Add next packet id to message and publish
                                        let PacketIdentifier(pkid) = self._next_pkid();
                                        message.set_pkid(pkid);
                                        let _ = self._publish(message);
                                        self.pub2_channel_pending -= 1;
                                    }
                                }
                            }
                        }
                    }
                    MioNotification::Sub => {
                        let topics = match self.sub_rx {
                            Some(ref sub_rx) => sub_rx.recv().unwrap(),
                            None => panic!("Expected a subscribe recieve channel"),
                        };
                        let _ = self._subscribe(topics);
                    }
                    MioNotification::Disconnect => {
                        debug!("{:?}", self.state);
                        match self.state {
                            MqttState::Connected => {
                                let _ = self._disconnect();
                                self.state = MqttState::Disconnected;
                            }
                            _ => debug!("Mqtt connection not established"),
                        }
                    }
                    MioNotification::Shutdown => {
                        let _ = self.stream.shutdown(Shutdown::Both);
                    }
                }
            }

            MqttState::Disconnected | MqttState::Handshake => {
                error!("cannot handle user request in handshake/disconnected state")
            }
        }
    }
}

impl MqttClient {
    pub fn new(opts: MqttOptions) -> Self {
        let addr = opts.addr;
        //TODO: Move state initialization to MqttClient constructor
        MqttClient {
            addr: addr.expect("No address in client options"),
            stream: NetworkStream::None,

            // State
            last_flush: Instant::now(),
            last_pkid: PacketIdentifier(0),
            await_ping: false,
            state: MqttState::Disconnected,
            initial_connect: true,
            opts: opts,
            pub1_channel_pending: 0,
            pub2_channel_pending: 0,
            should_qos1_block: false,
            should_qos2_block: false,
            no_of_reconnections: 0,

            // Channels
            pub0_rx: None,
            pub1_rx: None,
            pub2_rx: None,
            sub_rx: None,
            connsync_tx: None,
            mionotify_tx: None,

            // Queues
            incoming_rec: VecDeque::new(),
            outgoing_pub: VecDeque::new(),
            outgoing_rec: VecDeque::new(),
            outgoing_rel: VecDeque::new(),

            // callback
            callback: None,
        }
    }
    // Note: Setting callback before subscriber & publisher
    // are created ensures that message callbacks are registered
    // before subscription & you don't need to pass callbacks through
    // channels (simplifies code)
    pub fn message_callback<F>(mut self, callback: F) -> Self
        where F: Fn(Message) + Send + Sync + 'static
    {
        self.callback = Some(Arc::new(Box::new(callback)));
        self
    }

    /// Connects to the broker and starts an event loop in a new thread.
    /// Returns `Subscriber` and `Publisher` and handles reqests from them.
    /// Also handles network events, reconnections and retransmissions.
    pub fn start(mut self) -> Result<(Publisher, Subscriber)> {
        let mut event_loop = EventLoop::new().unwrap();
        let mionotify_tx = event_loop.channel();
        self.mionotify_tx = Some(mionotify_tx.clone());

        let (pub0_tx, pub0_rx) = mpsc::sync_channel::<Message>(self.opts.pub_q_len as usize);
        self.pub0_rx = Some(pub0_rx);
        let (pub1_tx, pub1_rx) = mpsc::sync_channel::<Message>(self.opts.pub_q_len as usize);
        self.pub1_rx = Some(pub1_rx);
        let (pub2_tx, pub2_rx) = mpsc::sync_channel::<Message>(self.opts.pub_q_len as usize);
        self.pub2_rx = Some(pub2_rx);

        let (sub_tx, sub_rx) = mpsc::sync_channel::<Vec<(TopicFilter, QualityOfService)>>(self.opts.sub_q_len as usize);
        self.sub_rx = Some(sub_rx);

        // synchronizes tcp connection. why ?
        // start() call should fail if there a problem creating initial tcp
        // connection & mqtt connection. Since connections are happening inside thread,
        // this method should be informed to return error instead of
        // (publisher, subscriber) in case connection fails.
        let (connsync_tx, connsync_rx) = mpsc::sync_channel::<MqttStatus>(1);
        self.connsync_tx = Some(connsync_tx);
        // @ Create 'publisher' and 'subscriber'
        // @ These are the handles using which user interacts with rumqtt.
        let publisher = Publisher {
            pub0_tx: pub0_tx,
            pub1_tx: pub1_tx,
            pub2_tx: pub2_tx,
            mionotify_tx: mionotify_tx.clone(),
            retain: false,
        };

        let subscriber = Subscriber {
            subscribe_tx: sub_tx,
            mionotify_tx: mionotify_tx,
        };

        // This is a non blocking call and almost never fails
        // Whether connection is successfu or not is to found
        // out in eventloop
        let stream = try!(TcpStream::connect(&self.addr));
        let stream = NetworkStream::Tcp(stream);
        self.stream = stream;

        //TODO: Handle thread death
        thread::spawn(move || {
            // State machine: Disconnected. Check if 'writable' success
            // to know connection success
            self.state = MqttState::Disconnected;
            event_loop.register(self.stream.get_ref().unwrap(),
                          MIO_CLIENT_STREAM,
                          EventSet::writable(),
                          PollOpt::edge() | PollOpt::oneshot())
                .unwrap();

            event_loop.run(&mut self).expect("Couldn't Run EventLoop");
        });
        let conn = connsync_rx.recv().expect("Connection sync recv error");
        match conn {
            MqttStatus::Success => Ok((publisher, subscriber)),
            MqttStatus::Failed => Err(Error::ConnectionAbort),
        } 
    }

    /// Return a count of (successful) mqtt connections that happened from the start.
    /// Just to know how many times the client reconnected (coz of bad networks, broker crashes etc)
    pub fn get_reconnection_count(&self) -> u32 {
        self.no_of_reconnections
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
                        match self.outgoing_pub
                            .iter()
                            .position(|ref x| x.1.get_pkid() == Some(pkid)) {
                            Some(i) => {
                                self.outgoing_pub.remove(i);
                            }
                            None => {
                                error!("Oopssss..unsolicited ack");
                            }
                        };
                        debug!("Pub Q Len After Ack @@@ {:?}", self.outgoing_pub.len());
                        Ok(HandlePacket::PubAck)
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
                                error!("Oopssss..unsolicited record");
                            }
                        };

                        try!(self._pubrel(pkid));
                        self.outgoing_rel.push_back((time::get_time().sec, PacketIdentifier(pkid)));
                        Ok(HandlePacket::PubRec)
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
                                error!("Oopssss..unsolicited release. Message might have already been released");
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
                                error!("Oopssss..unsolicited complete");
                                None
                            }
                        };
                        Ok(HandlePacket::PubComp)
                    }

                    VariablePacket::UnsubackPacket(..) => Ok(HandlePacket::UnSubAck),

                    _ => Ok(HandlePacket::Invalid), //TODO: Replace this with panic later
                }
            }

            MqttState::Disconnected => panic!("Invalid state while handling packets"),
        }
    }

    //TODO: Rename to handle incoming publish
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
            // @ if 'pubrec' is lost, broker will resend the message. so only pushback is pkid is new.
            // @ and resend pubcomp.
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
                Ok(HandlePacket::PubRec)
            }
        }
    }

    #[allow(non_snake_case)]
    fn STATE_read_incoming(&mut self, event_loop: &mut EventLoop<Self>) -> Result<VariablePacket> {
        match VariablePacket::decode(&mut self.stream) {
            // @ Decoded packet successfully.
            // @ Retrigger readable in evenloop
            Ok(pk) => Ok(pk),
            // @ Try to make a new Tcp connection.
            // @ Set state machine to Disconnected.
            // @ Make eventloop writable to check connection status
            Err(err) => {
                // maybe size=0 while reading indicating socket
                // close at broker end
                error!("Error in receiving packet {:?}", err);
                self._unbind();
                //TODO: Return actual error
                Err(Error::Read)
            }
        }
    }

    #[allow(non_snake_case)]
    fn STATE_handle_packet(&mut self, packet: &VariablePacket, event_loop: &mut EventLoop<Self>) {
        if let Ok(p) = self.handle_packet(packet) {
            match p {
                // Mqtt connection established, start the timers
                HandlePacket::ConnAck => {
                    self.state = MqttState::Connected;
                    self.no_of_reconnections += 1;
                    if self.initial_connect == true {
                        match self.connsync_tx {
                            Some(ref connsync_tx) => {
                                connsync_tx.send(MqttStatus::Success).expect("Send failed");
                                self.initial_connect = false;
                            }
                            None => panic!("No connsync channel"),
                        }
                    } else {
                        // Publisher won't stop even when disconnected until channel is full.
                        // This notifies notify() to publish channel pending messages after reconnect.
                        match self.mionotify_tx {
                            Some(ref mionotify_tx) => {
                                mionotify_tx.send(MioNotification::Pub(PubNotify::QoS1Reconnect)).expect("Send failed");
                                mionotify_tx.send(MioNotification::Pub(PubNotify::QoS2Reconnect)).expect("Send failed");
                            }
                            None => panic!("No mionotify channel"),
                        }
                    }

                    // TODO: Bug?? Multiple timers after restart?
                    event_loop.timeout_ms(MIO_QUEUE_TIMER, self.opts.queue_timeout as u64 * 1000).unwrap();
                    if let Some(keep_alive) = self.opts.keep_alive {
                        event_loop.timeout_ms(MIO_PING_TIMER, keep_alive as u64 * 900).unwrap();
                    }

                }
                HandlePacket::Publish(m) => {
                    if let Some(ref message_callback) = self.callback {
                        let message_callback = message_callback.clone();

                        // Have a thread pool to handle message callbacks. Take the threadpool as a parameter
                        thread::spawn(move || message_callback(*m));
                    }
                }
                // Sending a dummy notification saying tha queue size has reduced
                HandlePacket::PubAck => {
                    // Don't notify everytime q len is < max. This will always be true initially
                    // leading to dup notify.
                    // Send only for notify() to recover if channel is blocked.
                    // Blocking = true is set during publish if pub q len is more than desired.
                    if self.outgoing_pub.len() < self.opts.pub_q_len as usize && self.should_qos1_block == true {
                        match self.mionotify_tx {
                            Some(ref mionotify_tx) => {
                                self.should_qos1_block = false;
                                mionotify_tx.send(MioNotification::Pub(PubNotify::QoS1QueueDown)).expect("Send failed");
                            }
                            None => panic!("No mionotify channel"),
                        }
                    }
                }
                // TODO: Better read from channel again after PubComp instead of PubRec
                HandlePacket::PubRec => {
                    if self.outgoing_rec.len() < self.opts.pub_q_len as usize && self.should_qos2_block == true {
                        match self.mionotify_tx {
                            Some(ref mionotify_tx) => {
                                self.should_qos2_block = false;
                                mionotify_tx.send(MioNotification::Pub(PubNotify::QoS2QueueDown)).expect("Send failed");
                            }
                            None => panic!("No mionotify channel"),
                        }
                    }
                }
                _ => info!("packet handler says that he doesn't care"),
            }
        } else {
            error!("Error handling the packet");
        }
    }

    #[allow(non_snake_case)]
    fn STATE_try_reconnect(&mut self, event_loop: &mut EventLoop<Self>) {
        // Handles case where initial connection failures
        // E.g testsuite --> inital_mqtt_connect_failure_test
        if self.initial_connect == true {
            match self.connsync_tx {
                Some(ref connsync_tx) => {
                    connsync_tx.send(MqttStatus::Failed).expect("Send failed");
                    event_loop.shutdown();
                }
                None => panic!("No connsync channel"),
            }
            return;
        }
        let _ = self._try_reconnect();
    }

    fn _connect(&mut self) -> Result<()> {
        let connect = try!(self._generate_connect_packet());
        try!(self._write_packet(connect));
        self._flush()
    }

    pub fn _disconnect(&mut self) -> Result<()> {
        let disconnect = try!(self._generate_disconnect_packet());
        try!(self._write_packet(disconnect));
        self._flush()
    }


    fn _try_reconnect(&mut self) -> Result<()> {
        match self.opts.reconnect {
            //TODO: Implement
            None => panic!("To be implemented"),
            Some(dur) => {
                info!("  Will try Reconnect in {} seconds", dur);
                thread::sleep(Duration::new(dur as u64, 0));
                // Almost never fails
                match TcpStream::connect(&self.addr) {
                    Ok(stream) => self.stream = NetworkStream::Tcp(stream),
                    Err(err) => panic!("Error creating new stream {:?}", err),
                }
                Ok(())
            }
        }
    }

    fn _try_retransmit(&mut self) {
        match self.state {
            MqttState::Connected => {
                let outgoing_pub = self.outgoing_pub.clone(); //TODO: Remove the clone
                let outgoing_rec = self.outgoing_rec.clone(); //TODO: Remove the clone
                let outgoing_rel = self.outgoing_rel.clone(); //TODO: Remove the clone
                let timeout = self.opts.queue_timeout as i64;

                // Republish Qos 1 outgoing publishes
                for e in outgoing_pub.iter().filter(|ref x| time::get_time().sec - x.0 > timeout) {
                    let _ = self._publish(*e.1.clone());
                }

                // Republish QoS 2 outgoing records
                for e in outgoing_rec.iter().filter(|ref x| time::get_time().sec - x.0 > timeout) {
                    let _ = self._publish(*e.1.clone());
                }

                // Resend QoS 2 outgoing release
                for e in outgoing_rel.iter().filter(|ref x| time::get_time().sec - x.0 > timeout) {
                    let PacketIdentifier(pkid) = e.1;
                    let _ = self._pubrel(pkid);
                }
            }

            MqttState::Disconnected | MqttState::Handshake => error!("I won't republish. Client isn't in connected state"),
        }
    }

    fn ping(&mut self) -> Result<()> {
        let ping = try!(self._generate_pingreq_packet());
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
        let subscribe_packet = try!(self._generate_subscribe_packet(topics));
        try!(self._write_packet(subscribe_packet));
        self._flush()
    }

    fn _publish(&mut self, message: Message) -> Result<()> {
        let qos = message.qos;
        let message = message.transform(Some(qos));
        let payload = &*message.payload;
        let retain = message.retain;

        let publish_packet = try!(self._generate_publish_packet(message.topic.clone(), qos.clone(), retain, payload.clone()));

        match message.qos {
            QoSWithPacketIdentifier::Level0 => (),
            QoSWithPacketIdentifier::Level1(_) => {
                self.outgoing_pub.push_back((time::get_time().sec, message.clone()));
                if self.outgoing_pub.len() >= self.opts.pub_q_len as usize {
                    self.should_qos1_block = true;
                }
            }
            QoSWithPacketIdentifier::Level2(_) => {
                self.outgoing_rec.push_back((time::get_time().sec, message.clone()));
                if self.outgoing_rec.len() >= self.opts.pub_q_len as usize {
                    self.should_qos2_block = true;
                }
            }
        }
        debug!("       Publish {:?} {:?} > {} bytes", message.qos, message.topic.to_string(), message.payload.len());

        try!(self._write_packet(publish_packet));
        self._flush()
    }

    fn _puback(&mut self, pkid: u16) -> Result<()> {
        let puback_packet = try!(self._generate_puback_packet(pkid));
        try!(self._write_packet(puback_packet));
        self._flush()
    }

    fn _pubrec(&mut self, pkid: u16) -> Result<()> {
        let pubrec_packet = try!(self._generate_pubrec_packet(pkid));
        try!(self._write_packet(pubrec_packet));
        self._flush()
    }

    fn _pubrel(&mut self, pkid: u16) -> Result<()> {
        let pubrel_packet = try!(self._generate_pubrel_packet(pkid));
        try!(self._write_packet(pubrel_packet));
        self._flush()
    }

    fn _pubcomp(&mut self, pkid: u16) -> Result<()> {
        let puback_packet = try!(self._generate_pubcomp_packet(pkid));
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
        // broker.hivemq.com!("@@@ WRITING PACKET\n{:?}", packet);
        try!(self.stream.write_all(&packet));
        Ok(())
    }

    fn _generate_connect_packet(&self) -> Result<Vec<u8>> {
        let mut connect_packet = ConnectPacket::new("MQTT".to_owned(), self.opts.client_id.clone().unwrap());

        connect_packet.set_clean_session(self.opts.clean_session);

        if let Some(keep_alive) = self.opts.keep_alive {
            connect_packet.set_keep_alive(keep_alive);
        }

        // Converting (String, String) -> (TopicName, String)
        let will = match self.opts.will {
            Some(ref will) => Some((try!(TopicName::new(will.0.clone())), will.1.clone())),
            None => None,
        };

        if will.is_some() {
            connect_packet.set_will(will);
            connect_packet.set_will_qos(self.opts.will_qos as u8);
            connect_packet.set_will_retain(self.opts.will_retain);
        }

        // mqtt-protocol APIs are directly handling None cases.
        connect_packet.set_user_name(self.opts.username.clone());
        connect_packet.set_password(self.opts.password.clone());

        let mut buf = Vec::new();

        try!(connect_packet.encode(&mut buf));
        Ok(buf)
    }

    fn _generate_disconnect_packet(&self) -> Result<Vec<u8>> {
        let disconnect_packet = DisconnectPacket::new();
        let mut buf = Vec::new();

        try!(disconnect_packet.encode(&mut buf));
        Ok(buf)
    }

    fn _generate_pingreq_packet(&self) -> Result<Vec<u8>> {
        let pingreq_packet = PingreqPacket::new();
        let mut buf = Vec::new();

        try!(pingreq_packet.encode(&mut buf));
        Ok(buf)
    }

    fn _generate_subscribe_packet(&self, topics: Vec<(TopicFilter, QualityOfService)>) -> Result<Vec<u8>> {
        let subscribe_packet = SubscribePacket::new(11, topics);
        let mut buf = Vec::new();

        try!(subscribe_packet.encode(&mut buf));
        Ok(buf)
    }

    // TODO: dup flag
    fn _generate_publish_packet(&self,
                                topic: TopicName,
                                qos: QoSWithPacketIdentifier,
                                retain: bool,
                                payload: Vec<u8>)
                                -> Result<Vec<u8>> {
        let mut publish_packet = PublishPacket::new(topic, qos, payload);
        let mut buf = Vec::new();
        publish_packet.set_retain(retain);
        // publish_packet.set_dup(dup);
        try!(publish_packet.encode(&mut buf));
        Ok(buf)
    }

    fn _generate_puback_packet(&self, pkid: u16) -> Result<Vec<u8>> {
        let puback_packet = PubackPacket::new(pkid);
        let mut buf = Vec::new();

        try!(puback_packet.encode(&mut buf));
        Ok(buf)
    }

    fn _generate_pubrec_packet(&self, pkid: u16) -> Result<Vec<u8>> {
        let pubrec_packet = PubrecPacket::new(pkid);
        let mut buf = Vec::new();

        try!(pubrec_packet.encode(&mut buf));
        Ok(buf)
    }

    fn _generate_pubrel_packet(&self, pkid: u16) -> Result<Vec<u8>> {
        let pubrel_packet = PubrelPacket::new(pkid);
        let mut buf = Vec::new();

        try!(pubrel_packet.encode(&mut buf));
        Ok(buf)
    }

    fn _generate_pubcomp_packet(&self, pkid: u16) -> Result<Vec<u8>> {
        let pubcomp_packet = PubcompPacket::new(pkid);
        let mut buf = Vec::new();

        try!(pubcomp_packet.encode(&mut buf));
        Ok(buf)
    }

    #[inline]
    fn _next_pkid(&mut self) -> PacketIdentifier {
        let PacketIdentifier(pkid) = self.last_pkid;
        self.last_pkid = PacketIdentifier(pkid + 1);
        self.last_pkid
    }
}

/*
Why RuMqtt:
GOALS
-----
1. Synchronous mqtt connects: No need of callback to check if mqtt connection is
successful or not. You'll know of of errors (if any) synchronously
2. Synchronous subscribes (TODO): Same as above
3. Queued publishes: publishes won't throw errors by default. A queue (with user defined
    length) will be buffered when the n/w is down. If n/w is down for some time and queue
    becomes full, publishes are blocked
4. No locks. Fast and efficient because of Rust and Mio
5. Callback only for subscibed incoming message. Callbacks are executed using threadpool (mioco)
*/