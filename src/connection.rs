use std::net::{TcpStream, SocketAddr, Shutdown};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use std::thread;
use std::io::{Write, ErrorKind};
use std::collections::VecDeque;

use mqtt::packet::*;
use mqtt::Decodable;
use mqtt::control::variable_header::ConnectReturnCode;
use mqtt::control::fixed_header::FixedHeaderError;
use mqtt::control::variable_header::PacketIdentifier;
use mqtt::{QualityOfService, TopicFilter};
use threadpool::ThreadPool;

use error::{Result, Error};
use clientoptions::MqttOptions;
use stream::{NetworkStream, SslContext};
use genpack;
use message::Message;
use callbacks::MqttCallback;
// static mut N: i32 = 0;

enum HandlePacket {
    Publish(Box<Message>),
    PubAck(Option<Message>),
    PubRec(Option<Message>),
    PubComp,
    SubAck,
    UnSubAck,
    PingResp,
    Disconnect,
    None,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttState {
    Handshake,
    Connected,
    Disconnected,
}

#[derive(Debug)]
pub enum NetworkRequest {
    Subscribe(Vec<(TopicFilter, QualityOfService)>),
    Publish(Message),
    Shutdown,
    Disconnect,
}

pub struct Connection {
    pub addr: SocketAddr,
    pub domain: String,
    pub opts: MqttOptions,
    pub stream: NetworkStream,
    pub nw_request_rx: Receiver<NetworkRequest>,
    pub state: MqttState,
    pub initial_connect: bool,
    pub await_pingresp: bool,
    pub last_flush: Instant,

    pub last_pkid: PacketIdentifier,

    // Callbacks
    pub callback: Option<MqttCallback>,

    // Queues. Note: 'record' is qos2 term for 'publish'
    /// For QoS 1. Stores outgoing publishes
    pub outgoing_pub: VecDeque<(Box<Message>)>,
    /// For QoS 2. Store for incoming publishes to record.
    pub incoming_rec: VecDeque<Box<Message>>, //
    /// For QoS 2. Store for outgoing publishes.
    pub outgoing_rec: VecDeque<(Box<Message>)>,
    /// For Qos2. Store for outgoing `pubrel` packets.
    pub outgoing_rel: VecDeque<(PacketIdentifier)>,
    /// For Qos2. Store for outgoing `pubcomp` packets.
    pub outgoing_comp: VecDeque<(PacketIdentifier)>,

    // clean_session=false will remember subscriptions only till lives.
    // If broker crashes, all its state will be lost (most brokers).
    // client wouldn't want to loose messages after it comes back up again
    pub subscriptions: VecDeque<Vec<(TopicFilter, QualityOfService)>>,

    pub no_of_reconnections: u32,

    pub pool: ThreadPool,

    // Prevent reconnects when not desired
    pub dont_reconnect: bool,
}

impl Connection {
    pub fn connect(addr: SocketAddr,
                   opts: MqttOptions,
                   nw_request_rx: Receiver<NetworkRequest>,
                   callback: Option<MqttCallback>)
                   -> Result<Self> {

        let mut connection = Connection {
            addr: addr,
            domain: opts.addr.split(":").map(str::to_string).next().unwrap_or_default(),
            opts: opts,
            stream: NetworkStream::None,
            nw_request_rx: nw_request_rx,
            state: MqttState::Disconnected,
            initial_connect: true,
            await_pingresp: false,
            last_flush: Instant::now(),
            last_pkid: PacketIdentifier(0),

            callback: callback,

            // Queues
            incoming_rec: VecDeque::new(),
            outgoing_pub: VecDeque::new(),
            outgoing_rec: VecDeque::new(),
            outgoing_rel: VecDeque::new(),
            outgoing_comp: VecDeque::new(),

            // Subscriptions
            subscriptions: VecDeque::new(),

            no_of_reconnections: 0,

            // Threadpool
            pool: ThreadPool::new(1),

            // Prevent reconnects after disconnect
            dont_reconnect: false,
        };

        connection.state = MqttState::Disconnected;
        connection.try_reconnect()?;
        connection.state = MqttState::Handshake;
        connection.await_connack()?;
        connection.state = MqttState::Connected;
        info!("$$$ Connected to broker");
        connection.stream.set_read_timeout(Some(Duration::new(1, 0)))?;
        connection.stream.set_write_timeout(Some(Duration::new(10, 0)))?;
        Ok(connection)
    }

    pub fn run(&mut self) -> Result<()> {
        'reconnect: loop {
            loop {
                if self.initial_connect {
                    self.initial_connect = false;
                    break;
                } else {
                    // Don't reconnect if we've explicitly disconnected
                    if self.dont_reconnect {
                        info!("$$$ Terminating due to explicit user request");
                        return Ok(())
                    }

                    self.state = MqttState::Disconnected;
                    match self.try_reconnect() {
                        Ok(_) => {
                            self.state = MqttState::Handshake;
                            let packet = self.await_connack()?;
                            self.state = MqttState::Connected;
                            info!("$$$ Connected to broker");
                            self.post_connack_handle(&packet)?;
                            self.stream.set_read_timeout(Some(Duration::new(1, 0)))?;
                            break;
                        }
                        Err(e) => {
                            error!("Couldn't connect. Error = {:?}", e);
                            error!("  Will try Reconnect in {:?} seconds", self.opts.reconnect);
                            continue;
                        }
                    }
                }
            }

            'receive: loop {
                let packet = match VariablePacket::decode(&mut self.stream) {
                    // @ Decoded packet successfully.
                    Ok(pk) => pk,
                    Err(err) => {
                        match err {
                            VariablePacketError::FixedHeaderError(err) => {
                                if let FixedHeaderError::IoError(err) = err {
                                    match err.kind() {
                                        // Timedout for Windows, WouldBlock for linux
                                        ErrorKind::TimedOut | ErrorKind::WouldBlock => {
                                            // During timeout write errors when tcp write
                                            // buffers are full, ping below will error out because of
                                            // PingTimeout when (n * write timeout) > ping timeout

                                            // TODO: Test if PINGRESPs are properly recieved before
                                            // next ping incase of high frequency incoming messages
                                            if let Err(e) = self.ping() {
                                                error!("PING error {:?}", e);
                                                self.unbind();
                                                continue 'reconnect;
                                            }

                                            // NOTE: Force pings during Write timeout/ Queue overflow is
                                            // bad because unlike idle state, publishes might be continous
                                            // and connection might not go into read mod for detecting
                                            // pingreps before consequent publishes
                                            let _ = self.write();

                                            continue 'receive;
                                        }
                                        _ => {
                                            // Socket error are readily available here as soon as
                                            // broker closes its socket end. (But not inbetween n/w disconnection
                                            // and socket close at broker [i.e ping req timeout])
                                            // UPDATE: Lot of publishes are being written by the time this notified
                                            // the eventloop thread. Setting disconnect_block = true during write failure
                                            error!("At line {:?} = Error in receiving packet {:?}", line!(), err);
                                            self.unbind();
                                            continue 'reconnect;
                                        }
                                    }
                                } else {
                                    error!("At line {:?} = Error reading packet = {:?}", line!(), err);
                                    self.unbind();
                                    continue 'reconnect;
                                }
                            }
                            _ => {
                                error!("At line {:?} = Error reading packet = {:?}", line!(), err);
                                self.unbind();
                                continue 'reconnect;
                            }
                        }
                    }
                };

                if let Err(e) = self.post_handle_packet(&packet) {
                    error!("At line {:?} = Error handling packet = {:?}", line!(), e);
                    continue 'receive;
                }
            }
        }
    }

    // http://stackoverflow.
    // com/questions/11115364/mqtt-messageid-practical-implementation
    #[inline]
    fn next_pkid(&mut self) -> PacketIdentifier {
        let PacketIdentifier(mut pkid) = self.last_pkid;
        if pkid == 65535 {
            pkid = 0;
        }
        self.last_pkid = PacketIdentifier(pkid + 1);
        self.last_pkid
    }

    fn write(&mut self) -> Result<()> {
        // @ Only read from `Network Request` channel when connected. Or else Empty
        // return.
        // @ Helps in case where Tcp connection happened but in MqttState::Handshake
        // state.
        if self.state == MqttState::Connected {
            for _ in 0..50 {
                match self.nw_request_rx.try_recv()? {
                    NetworkRequest::Shutdown => {
                        self.dont_reconnect = true;
                        self.stream.shutdown(Shutdown::Both)?
                    },
                    NetworkRequest::Disconnect => {
                        self.dont_reconnect = true;
                        self.disconnect()?
                    },
                    NetworkRequest::Publish(m) => self.publish(m)?,
                    NetworkRequest::Subscribe(s) => {
                        self.subscriptions.push_back(s.clone());
                        self.subscribe(s)?;
                    }
                };
            }
        }
        Ok(())
    }

    fn try_reconnect(&mut self) -> Result<()> {
        if !self.initial_connect {
            thread::sleep(Duration::new(self.opts.reconnect as u64, 0));
        }
        let stream = TcpStream::connect(&self.addr)?;
        let stream = match self.opts.ca {
            Some(ref ca) => {
                if let Some((ref crt, ref key)) = self.opts.client_cert {
                    let ssl_ctx: SslContext = SslContext::new(ca, Some((crt, key)), self.opts.verify_ca)?;
                    NetworkStream::Tls(ssl_ctx.connect(&self.domain, stream)?)
                } else {
                    let ssl_ctx: SslContext = SslContext::new(ca, None::<(String, String)>, self.opts.verify_ca)?;
                    NetworkStream::Tls(ssl_ctx.connect(&self.domain, stream)?)
                }
            }
            None => NetworkStream::Tcp(stream),
        };

        self.stream = stream;
        let connect = genpack::generate_connect_packet(self.opts.clone())?;
        self.write_packet(connect)?;
        Ok(())
    }

    fn await_connack(&mut self) -> Result<VariablePacket> {
        let packet = VariablePacket::decode(&mut self.stream).map_err(|_| Error::InvalidPacket)?;
        match self.state {
            MqttState::Handshake => {
                match packet {
                    VariablePacket::ConnackPacket(ref connack) => {
                        let conn_ret_code = connack.connect_return_code();
                        if conn_ret_code != ConnectReturnCode::ConnectionAccepted {
                            error!("Failed to connect, err {:?}", conn_ret_code);
                            return Err(Error::ConnectionRefused(conn_ret_code));
                        }
                    }
                    _ => {
                        error!("received invalid packet in handshake state --> {:?}", packet);
                        return Err(Error::InvalidPacket);
                    }
                }
                Ok(packet)
            }

            MqttState::Disconnected | MqttState::Connected => {
                error!("Invalid State during CONNACK packet");
                Err(Error::InvalidState)
            }
        }
    }

    fn post_connack_handle(&mut self, packet: &VariablePacket) -> Result<()> {
        match self.state {
            MqttState::Connected => {
                match *packet {
                    VariablePacket::ConnackPacket(..) => {
                        self.no_of_reconnections += 1;
                        if self.opts.clean_session {
                            // Resubscribe after a reconnection when connected with clean session.
                            for s in self.subscriptions.clone() {
                                let _ = self.subscribe(s);
                            }
                        }

                        // Retransmit QoS1,2 queues after reconnection when clean_session = false
                        if !self.opts.clean_session {
                            self.force_retransmit();
                        }
                        Ok(())
                    }
                    _ => {
                        error!("Invalid Packet in HandShake State During Connack Handling");
                        Err(Error::HandshakeFailed)
                    }
                }
            }
            _ => {
                error!("Invaild State While Handling Connack Packet");
                Err(Error::ConnectionAbort)
            }
        }
    }

    fn post_handle_packet(&mut self, packet: &VariablePacket) -> Result<()> {
        let handle = self.handle_packet(packet)?;
        match handle {
            HandlePacket::Publish(m) => {
                if let Some(ref callback) = self.callback {
                    if let Some(ref on_message) = callback.on_message {
                        let on_message = on_message.clone();
                        self.pool.execute(move || on_message(*m));
                    }
                }
            }
            HandlePacket::PubAck(m) => {
                if let Some(val) = m {
                    if let Some(ref callback) = self.callback {
                        if let Some(ref on_publish) = callback.on_publish {
                            let on_publish = on_publish.clone();
                            self.pool.execute(move || on_publish(val));
                        }
                    }

                }
            }
            // TODO: Better read from channel again after PubComp instead of PubRec
            HandlePacket::PubRec(m) => {
                if let Some(val) = m {
                    if let Some(ref callback) = self.callback {
                        if let Some(ref on_publish) = callback.on_publish {
                            let on_publish = on_publish.clone();
                            self.pool.execute(move || on_publish(val));
                        }
                    }
                }
            }
            _ => debug!("packet handler says that he doesn't care"),
        }
        Ok(())
    }

    fn handle_puback(&mut self, puback: &PubackPacket) -> Result<HandlePacket> {
        let pkid = puback.packet_identifier();
        debug!("*** PubAck --> Pkid({:?})\n--- Publish Queue =\n{:#?}\n\n", pkid, self.outgoing_pub);
        let m = match self.outgoing_pub
            .iter()
            .position(|x| x.get_pkid() == Some(pkid)) {
            Some(i) => {
                if let Some(m) = self.outgoing_pub.remove(i) {
                    Some(*m)
                } else {
                    None
                }
            }
            None => {
                error!("Oopssss..unsolicited ack --> {:?}\n", puback);
                None
            }
        };
        debug!("Pub Q Len After Ack @@@ {:?}", self.outgoing_pub.len());
        Ok(HandlePacket::PubAck(m))
    }

    fn handle_pubrec(&mut self, pubrec: &PubrecPacket) -> Result<HandlePacket> {
        let pkid = pubrec.packet_identifier();
        debug!("*** PubRec --> Pkid({:?})\n--- Record Queue =\n{:#?}\n\n", pkid, self.outgoing_rec);
        let m = match self.outgoing_rec
            .iter()
            .position(|x| x.get_pkid() == Some(pkid)) {
            Some(i) => {
                if let Some(m) = self.outgoing_rec.remove(i) {
                    Some(*m)
                } else {
                    None
                }
            }
            None => {
                error!("Oopssss..unsolicited record --> {:?}", pubrec);
                None
            }
        };

        // After receiving PUBREC packet and removing corrosponding
        // message from outgoing_rec queue, send PUBREL and add it queue.
        self.outgoing_rel.push_back(PacketIdentifier(pkid));
        // NOTE: Don't Error return here. It's ok to fail during writes coz of
        // disconnection.
        // `force_transmit` will resend when reconnection is successful
        let _ = self.pubrel(pkid);
        Ok(HandlePacket::PubRec(m))

    }

    fn handle_pubrel(&mut self, pubrel: &PubrelPacket) -> Result<HandlePacket> {
        let pkid = pubrel.packet_identifier();
        let message = match self.incoming_rec
            .iter()
            .position(|x| x.get_pkid() == Some(pkid)) {
            Some(i) => {
                if let Some(message) = self.incoming_rec.remove(i) {
                    self.outgoing_comp.push_back(PacketIdentifier(pkid));
                    let _ = self.pubcomp(pkid);
                    Some(message)
                } else {
                    None
                }
            }
            None => {
                error!("Oopssss..unsolicited release. Message might have already been released --> {:?}", pubrel);
                None
            }
        };

        self.outgoing_comp.push_back(PacketIdentifier(pkid));
        let _ = self.pubcomp(pkid);

        if let Some(message) = message {
            Ok(HandlePacket::Publish(message))
        } else {
            Ok(HandlePacket::Invalid)
        }
    }

    fn handle_pubcomp(&mut self, pubcomp: &PubcompPacket) -> Result<HandlePacket> {
        let pkid = pubcomp.packet_identifier();
        match self.outgoing_rel
            .iter()
            .position(|x| *x == PacketIdentifier(pkid)) {
            Some(pos) => self.outgoing_rel.remove(pos),
            None => {
                error!("Oopssss..unsolicited complete --> {:?}", pubcomp);
                None
            }
        };
        Ok(HandlePacket::PubComp)
    }

    fn handle_packet(&mut self, packet: &VariablePacket) -> Result<HandlePacket> {
        match self.state {
            MqttState::Connected => {
                match *packet {
                    VariablePacket::SubackPacket(..) => Ok(HandlePacket::SubAck),
                    VariablePacket::PingrespPacket(..) => {
                        self.await_pingresp = false;
                        Ok(HandlePacket::PingResp)
                    }
                    VariablePacket::DisconnectPacket(..) => Ok(HandlePacket::Disconnect),
                    VariablePacket::PubackPacket(ref puback) => self.handle_puback(puback),
                    VariablePacket::PublishPacket(ref publ) => self.handle_message(Message::from_pub(publ)?),
                    // @ Qos2 message published by client is recorded by broker
                    // @ Remove message from 'outgoing_rec' queue and add pkid to 'outgoing_rel'
                    // @ Send 'pubrel' to broker
                    VariablePacket::PubrecPacket(ref pubrec) => self.handle_pubrec(pubrec),
                    // @ Broker knows that client has the message
                    // @ release the message stored in 'recorded' queue
                    // @ send 'pubcomp' to sender indicating that message is released
                    // @ if 'pubcomp' packet is lost, broker will send pubrel again
                    // @ for the released message, for which we send dummy 'pubcomp' again
                    VariablePacket::PubrelPacket(ref pubrel) => self.handle_pubrel(pubrel),
                    // @ Remove this pkid from 'outgoing_rel' queue
                    VariablePacket::PubcompPacket(ref pubcomp) => self.handle_pubcomp(pubcomp),

                    VariablePacket::UnsubackPacket(..) => Ok(HandlePacket::UnSubAck),
                    _ => {
                        error!("Invalid Packet in Connected State --> {:?}", packet);
                        Ok(HandlePacket::Invalid)
                    }
                }
            }
            MqttState::Disconnected | MqttState::Handshake => {
                error!("Invalid ({:?}) State While Handling Packet --> {:?}", self.state, packet);
                Err(Error::ConnectionAbort)
            }
        }
    }

    // TODO: Rename to handle incoming publish
    fn handle_message(&mut self, message: Box<Message>) -> Result<HandlePacket> {
        debug!("       Publish {:?} {:?} < {:?} bytes",
               message.qos,
               message.topic.to_string(),
               message.payload.len());
        match message.qos {
            QoSWithPacketIdentifier::Level0 => Ok(HandlePacket::Publish(message)),
            QoSWithPacketIdentifier::Level1(pkid) => {
                self.puback(pkid)?;
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

                self.pubrec(pkid)?;
                Ok(HandlePacket::None)
            }
        }
    }

    // Spec says that client (for QoS > 0, persistant session [clean session = 0])
    // should retransmit all the unacked publishes and pubrels after reconnection.
    // NOTE: Sending duplicate pubrels isn't a problem (I guess ?). Broker will
    // just resend pubcomps
    fn force_retransmit(&mut self) {
        // Cloning because iterating and removing isn't possible.
        // Iterating over indexes and and removing elements messes
        // up the remove sequence
        let mut outgoing_pub = self.outgoing_pub.clone();
        debug!("*** Force Retransmission. Publish Queue =\n{:#?}\n\n", outgoing_pub);
        self.outgoing_pub.clear();
        while let Some(message) = outgoing_pub.pop_front() {
            let _ = self.publish(*message);
        }

        let mut outgoing_rec = self.outgoing_rec.clone();
        debug!("*** Force Retransmission. Record Queue =\n{:#?}\n\n", outgoing_rec);
        self.outgoing_rec.clear();
        while let Some(message) = outgoing_rec.pop_front() {
            let _ = self.publish(*message);
        }
        // println!("{:?}", self.outgoing_rec.iter().map(|e|
        // e.1.qos).collect::<Vec<_>>());
        let mut outgoing_rel = self.outgoing_rel.clone();
        self.outgoing_rel.clear();
        while let Some(rel) = outgoing_rel.pop_front() {
            self.outgoing_rel.push_back(rel);
            let PacketIdentifier(pkid) = rel;
            let _ = self.pubrel(pkid);
        }
    }

    pub fn disconnect(&mut self) -> Result<()> {
        let disconnect = genpack::generate_disconnect_packet()?;
        self.write_packet(disconnect)?;
        Ok(())
    }

    fn subscribe(&mut self, topics: Vec<(TopicFilter, QualityOfService)>) -> Result<()> {
        let subscribe_packet = genpack::generate_subscribe_packet(topics)?;
        self.write_packet(subscribe_packet)?;
        Ok(())
    }

    fn publish(&mut self, mut message: Message) -> Result<()> {
        let PacketIdentifier(pkid) = self.next_pkid();
        message.set_pkid(pkid);
        let qos = message.qos;
        let message_box = message.to_boxed(Some(qos));
        let payload_len = message.payload.len();

        let publish_packet = {
            let payload = &*message.payload;
            let topic = message.topic;
            let retain = message.retain;
            genpack::generate_publish_packet(topic, message.qos, retain, payload.to_vec())?
        };

        let mut size_exceeded = false;

        match message.qos {
            QoSWithPacketIdentifier::Level0 => (),
            QoSWithPacketIdentifier::Level1(_) => {
                if payload_len > self.opts.storepack_sz {
                    size_exceeded = true;
                    warn!("Dropping packets due to size limit exceeded");
                } else {
                    self.outgoing_pub.push_back(message_box.clone());
                }

                if self.outgoing_pub.len() > self.opts.pub_q_len as usize * 50 {
                    warn!(":( :( Outgoing Publish Queue Length growing bad --> {:?}", self.outgoing_pub.len());
                }
            }
            QoSWithPacketIdentifier::Level2(_) => {
                if payload_len > self.opts.storepack_sz {
                    size_exceeded = true;
                    warn!("Dropping packets due to size limit exceeded");
                } else {
                    self.outgoing_rec.push_back(message_box.clone());
                }

                if self.outgoing_rec.len() > self.opts.pub_q_len as usize * 50 {
                    warn!(":( :( Outgoing Record Queue Length growing bad --> {:?}", self.outgoing_rec.len());
                }
            }
        }

        match message.qos {
            QoSWithPacketIdentifier::Level0 if !size_exceeded => self.write_packet(publish_packet)?,
            QoSWithPacketIdentifier::Level1(_) |
            QoSWithPacketIdentifier::Level2(_) if !size_exceeded => {
                if self.state == MqttState::Connected {
                    self.write_packet(publish_packet)?;
                } else {
                    warn!("State = {:?}. Skip network write", self.state);
                }
            }
            _ => {}
        }

        // error!("Queue --> {:?}\n\n", self.outgoing_pub);
        // debug!("       Publish {:?} {:?} > {} bytes", message.qos,
        // topic.clone().to_string(), message.payload.len());
        Ok(())
    }

    fn ping(&mut self) -> Result<()> {
        // debug!("client state --> {:?}, await_ping --> {}", self.state,
        // self.await_ping);

        match self.state {
            MqttState::Connected => {
                if let Some(keep_alive) = self.opts.keep_alive {
                    let elapsed = self.last_flush.elapsed();

                    if elapsed >= Duration::from_millis(((keep_alive * 1000) as f64 * 0.9) as u64) {
                        if elapsed >= Duration::new((keep_alive + 1) as u64, 0) {
                            return Err(Error::PingTimeout);
                        }

                        // @ Prevents half open connections. Tcp writes will buffer up
                        // with out throwing any error (till a timeout) when internet
                        // is down. Eventhough broker closes the socket, EOF will be
                        // known only after reconnection.
                        // We just unbind the socket if there in no pingresp before next ping
                        // (What about case when pings aren't sent because of constant publishes
                        // ?. A. Tcp write buffer gets filled up and write will be blocked for 10
                        // secs and then error out because of timeout.)
                        if self.await_pingresp {
                            return Err(Error::AwaitPingResp);
                        }

                        let ping = genpack::generate_pingreq_packet()?;
                        self.await_pingresp = true;
                        self.write_packet(ping)?;
                    }
                }
            }

            MqttState::Disconnected | MqttState::Handshake => error!("I won't ping. Client is in disconnected/handshake state"),
        }
        Ok(())
    }

    fn puback(&mut self, pkid: u16) -> Result<()> {
        let puback_packet = genpack::generate_puback_packet(pkid)?;
        self.write_packet(puback_packet)?;
        Ok(())
    }

    fn pubrec(&mut self, pkid: u16) -> Result<()> {
        let pubrec_packet = genpack::generate_pubrec_packet(pkid)?;
        self.write_packet(pubrec_packet)?;
        Ok(())
    }

    fn pubrel(&mut self, pkid: u16) -> Result<()> {
        let pubrel_packet = genpack::generate_pubrel_packet(pkid)?;
        self.write_packet(pubrel_packet)?;
        Ok(())
    }

    fn pubcomp(&mut self, pkid: u16) -> Result<()> {
        let puback_packet = genpack::generate_pubcomp_packet(pkid)?;
        self.write_packet(puback_packet)?;
        Ok(())
    }

    fn unbind(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
        self.await_pingresp = false;
        self.state = MqttState::Disconnected;

        // remove all the state
        if self.opts.clean_session {
            self.outgoing_pub.clear();
            self.outgoing_rec.clear();
            self.outgoing_rel.clear();
            self.outgoing_comp.clear();
        }

        error!("  Disconnected {:?}", self.opts.client_id);
    }

    // NOTE: write_all() will block indefinitely by default if
    // underlying Tcp Buffer is full (during disconnections). This
    // is evident when test cases are publishing lot of data when
    // ethernet cable is unplugged (mantests/half_open_publishes_and_reconnections
    // but not during mantests/ping_reqs_in_time_and_reconnections due to low
    // frequency writes. 10 seconds migth be good default for write timeout ?)

    #[inline]
    fn write_packet(&mut self, packet: Vec<u8>) -> Result<()> {
        if let Err(e) = self.stream.write_all(&packet){
            warn!("{:?}", e);
            return Err(e.into());
        }
        self.flush()?;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.stream.flush()?;
        self.last_flush = Instant::now();
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::{Connection, MqttState};
    use clientoptions::MqttOptions;
    use mqtt::control::variable_header::PacketIdentifier;

    use std::net::{SocketAddr, ToSocketAddrs};
    use std::collections::VecDeque;
    use std::time::Instant;
    use std::sync::mpsc::sync_channel;

    use threadpool::ThreadPool;
    use stream::NetworkStream;

    pub fn mock_connect() -> Connection {
        fn lookup_ipv4<A: ToSocketAddrs>(addr: A) -> SocketAddr {
            let addrs = addr.to_socket_addrs().expect("Conversion Failed");
            for addr in addrs {
                if let SocketAddr::V4(_) = addr {
                    return addr;
                }
            }
            unreachable!("Cannot lookup address");
        }

        let addr = lookup_ipv4("test.mosquitto.org:1883");
        let (_, rx) = sync_channel(10);
        let opts = MqttOptions::new();
        let domain = opts.addr.split(":").map(str::to_string).next().unwrap_or_default();
        let conn = Connection {
            addr: addr,
            opts: opts,
            domain: domain,
            stream: NetworkStream::None,
            nw_request_rx: rx,
            state: MqttState::Disconnected,
            initial_connect: true,
            await_pingresp: false,
            last_flush: Instant::now(),
            last_pkid: PacketIdentifier(0),
            callback: None,
            // Queues
            incoming_rec: VecDeque::new(),
            outgoing_pub: VecDeque::new(),
            outgoing_rec: VecDeque::new(),
            outgoing_rel: VecDeque::new(),
            outgoing_comp: VecDeque::new(),
            // Subscriptions
            subscriptions: VecDeque::new(),
            no_of_reconnections: 0,
            // Threadpool
            pool: ThreadPool::new(1),
        };
        conn
    }

    #[test]
    fn next_pkid_roll() {
        let mut connection = mock_connect();
        let mut pkt_id = PacketIdentifier(0);
        for _ in 0..65536 {
            pkt_id = connection.next_pkid();
        }
        assert_eq!(PacketIdentifier(1), pkt_id);
    }
}
