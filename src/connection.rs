use std::net::{TcpStream, SocketAddr, Shutdown};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use std::thread;
use std::io::{Write, ErrorKind};
use std::sync::Arc;
use std::collections::VecDeque;

use mqtt::packet::*;
use mqtt::Decodable;
use mqtt::control::variable_header::ConnectReturnCode;
use mqtt::control::fixed_header::FixedHeaderError;
use mqtt::control::variable_header::PacketIdentifier;
use mqtt::{QualityOfService, TopicFilter};
use mio::channel::SyncSender;
use threadpool::ThreadPool;
use time;

use error::{Result, Error};
use clientoptions::MqttOptions;
use tls::{NetworkStream, SslContext};
use genpack;
use message::Message;

static mut N: i32 = 0;

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
    Retransmit,
    Shutdown,
    Disconnect,
}

#[derive(Debug)]
pub enum NetworkNotification {
    Disconnected,
    Connected,
}

pub type MessageSendableFn = Box<Fn(Message) + Send + Sync>;
pub type PublishSendableFn = Box<Fn(Message) + Send + Sync>;

pub struct Connection {
    pub addr: SocketAddr,
    pub opts: MqttOptions,
    pub stream: NetworkStream,
    pub nw_request_rx: Receiver<NetworkRequest>,
    pub nw_notification_tx: SyncSender<NetworkNotification>,
    pub state: MqttState,
    pub initial_connect: bool,
    pub last_flush: Instant,

    /// On message callback
    pub message_callback: Option<Arc<MessageSendableFn>>,
    /// On publish callback
    pub publish_callback: Option<Arc<PublishSendableFn>>,

    // Queues. Note: 'record' is qos2 term for 'publish'
    /// For QoS 1. Stores outgoing publishes
    pub outgoing_pub: VecDeque<(i64, Box<Message>)>,
    /// For QoS 2. Store for incoming publishes to record.
    pub incoming_rec: VecDeque<Box<Message>>, //
    /// For QoS 2. Store for outgoing publishes.
    pub outgoing_rec: VecDeque<(i64, Box<Message>)>,
    /// For Qos2. Store for outgoing `pubrel` packets.
    pub outgoing_rel: VecDeque<(i64, PacketIdentifier)>,
    /// For Qos2. Store for outgoing `pubcomp` packets.
    pub outgoing_comp: VecDeque<(i64, PacketIdentifier)>,

    // clean_session=false will remember subscriptions only till lives.
    // If broker crashes, all its state will be lost (most brokers).
    // client wouldn't want to loose messages after it comes back up again
    pub subscriptions: VecDeque<Vec<(TopicFilter, QualityOfService)>>,

    pub no_of_reconnections: u32,

    pub pool: ThreadPool,
}

impl Connection {
    pub fn start(addr: SocketAddr,
                 opts: MqttOptions,
                 nw_request_rx: Receiver<NetworkRequest>,
                 nw_notification_tx: SyncSender<NetworkNotification>,
                 publish_callback: Option<Arc<PublishSendableFn>>,
                 message_callback: Option<Arc<MessageSendableFn>>)
                 -> Result<Self> {

        let mut connection = Connection {
            addr: addr,
            opts: opts,
            stream: NetworkStream::None,
            nw_request_rx: nw_request_rx,
            nw_notification_tx: nw_notification_tx,
            state: MqttState::Disconnected,
            initial_connect: true,
            last_flush: Instant::now(),

            publish_callback: publish_callback,
            message_callback: message_callback,

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
            pool: ThreadPool::new(4),
        };

        connection.state = MqttState::Disconnected;
        try!(connection._try_reconnect());
        connection.state = MqttState::Handshake;
        try!(connection._await_connack());
        connection.state = MqttState::Connected;
        try!(connection.nw_notification_tx.send(NetworkNotification::Connected));
        try!(connection.stream.set_read_timeout(Some(Duration::new(1, 0))));
        Ok(connection)
    }

    pub fn run(&mut self) -> Result<()> {
        'reconnect: loop {
            loop {
                if self.initial_connect {
                    self.initial_connect = false;
                    break;
                } else {
                    self.state = MqttState::Disconnected;
                    try!(self.nw_notification_tx.send(NetworkNotification::Disconnected));
                    match self._try_reconnect() {
                        Ok(_) => {
                            self.state = MqttState::Handshake;
                            let packet = try!(self._await_connack());
                            self.state = MqttState::Connected;
                            try!(self.post_connack_handle(&packet));
                            try!(self.nw_notification_tx.send(NetworkNotification::Connected));
                            try!(self.stream.set_read_timeout(Some(Duration::new(1, 0))));
                            break;
                        }
                        Err(_) => {
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
                                        ErrorKind::TimedOut | ErrorKind::WouldBlock => {
                                            let _ = self.write();
                                            if let Err(e) = self.ping() {
                                                match e {
                                                    Error::Timeout => {
                                                        error!("Couldn't PING in time :( . Err = {:?}", e);
                                                        self._unbind();
                                                        continue 'reconnect;
                                                    }
                                                    _ => {
                                                        error!("Error PINGING . Err = {:?}", e);
                                                        self._unbind();
                                                        continue 'reconnect;
                                                    }
                                                }
                                            }
                                            continue 'receive;
                                        }
                                        _ => {
                                            // Socket error are readily available here as soon as
                                            // disconnection happens. So it might be right for this
                                            // thread to ask for reconnection rather than reconnecting
                                            // during write failures
                                            // UPDATE: Lot of publishes are being written by the time this notified
                                            // the eventloop thread. Setting disconnect_block = true during write failure
                                            error!("Error in receiving packet {:?}", err);
                                            self._unbind();
                                            continue 'reconnect;
                                        }
                                    }
                                } else {
                                    error!("Error reading packet = {:?}", err);
                                    self._unbind();
                                    continue 'reconnect;
                                }
                            }
                            _ => {
                                error!("Error reading packet = {:?}", err);
                                self._unbind();
                                continue 'reconnect;
                            }
                        }
                    }
                };
                if let Err(e) = self.post_handle_packet(&packet) {
                    continue 'receive;
                }
            }
        }
    }

    fn ping(&mut self) -> Result<()> {
        // debug!("client state --> {:?}, await_ping --> {}", self.state,
        // self.await_ping);
        match self.state {
            MqttState::Connected => {
                if let Some(keep_alive) = self.opts.keep_alive {
                    let elapsed = self.last_flush.elapsed();
                    if elapsed >= Duration::new(keep_alive as u64, 0) {
                        return Err(Error::Timeout);
                    } else if elapsed >= Duration::new((keep_alive as f32 * 0.8) as u64, 0) {
                        try!(self._ping());
                    }
                }
            }

            MqttState::Disconnected | MqttState::Handshake => error!("I won't ping. Client is in disconnected/handshake state"),
        }
        Ok(())
    }

    fn write(&mut self) -> Result<()> {
        // @ Only read from `Network Request` channel when connected. Or else Empty
        // return.
        // @ Helps in case where Tcp connection happened but in MqttState::Handshake
        // state.
        if self.state == MqttState::Connected {
            for _ in 0..1000 {
                match try!(self.nw_request_rx.try_recv()) {
                    NetworkRequest::Shutdown => try!(self.stream.shutdown(Shutdown::Both)),
                    NetworkRequest::Disconnect => try!(self._disconnect()),
                    NetworkRequest::Retransmit => try!(self._try_retransmit()),
                    NetworkRequest::Publish(m) => try!(self._publish(m)),
                    NetworkRequest::Subscribe(s) => {
                        self.subscriptions.push_back(s.clone());
                        try!(self._subscribe(s));
                    }
                };
            }
        }
        Ok(())
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
                            let ssl_ctx: SslContext = try!(SslContext::new(ca, Some((crt, key)), self.opts.verify_ca));
                            NetworkStream::Tls(try!(ssl_ctx.connect(stream)))
                        } else {
                            let ssl_ctx: SslContext = try!(SslContext::new(ca, None::<(String, String)>, self.opts.verify_ca));
                            NetworkStream::Tls(try!(ssl_ctx.connect(stream)))
                        }
                    }
                    None => NetworkStream::Tcp(stream),
                };

                self.stream = stream;
                try!(self._connect());
                Ok(())
            }
        }
    }

    fn _await_connack(&mut self) -> Result<VariablePacket> {
        let packet = match VariablePacket::decode(&mut self.stream) {
            Ok(pk) => pk,
            Err(err) => {
                error!("Couldn't decode incoming packet. Error = {:?}", err);
                return Err(Error::InvalidPacket);
            }
        };

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
                return Ok(packet);
            }

            MqttState::Disconnected | MqttState::Connected => {
                error!("Invalid State during CONNACK packet");
                return Err(Error::InvalidState);
            }
        }
    }

    fn post_connack_handle(&mut self, packet: &VariablePacket) -> Result<()> {
        match self.state {
            MqttState::Connected => {
                match *packet {
                    VariablePacket::ConnackPacket(..) => {
                        self.no_of_reconnections += 1;
                        // Resubscribe after a reconnection.
                        for s in self.subscriptions.clone() {
                            let _ = self._subscribe(s);
                        }
                        // Retransmit QoS1,2 queues after reconnection. Clears the queue by the time
                        // QoS*Reconnect notifications are sent to read pending messages in the channel
                        self._force_retransmit();
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
        let handle = try!(self.handle_packet(packet));
        match handle {
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
                if self.outgoing_pub.len() > self.opts.pub_q_len as usize * 50 {
                    error!(":( :( Outgoing Publish Queue Length growing bad --> {:?}", self.outgoing_pub.len());
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
            HandlePacket::PubRec(m) => {
                if self.outgoing_rec.len() > self.opts.pub_q_len as usize * 50 {
                    error!(":( :( Outgoing Publish Queue Length growing bad");
                }

                if m.is_none() {
                    return Ok(());
                }

                if let Some(ref publish_callback) = self.publish_callback {
                    let publish_callback = publish_callback.clone();
                    self.pool.execute(move || publish_callback(m.unwrap()));
                }
            }
            _ => debug!("packet handler says that he doesn't care"),
        }
        Ok(())
    }

    fn handle_packet(&mut self, packet: &VariablePacket) -> Result<HandlePacket> {
        match self.state {
            MqttState::Connected => {
                match *packet {
                    VariablePacket::SubackPacket(..) => Ok(HandlePacket::SubAck),

                    VariablePacket::PingrespPacket(..) => Ok(HandlePacket::PingResp),

                    VariablePacket::DisconnectPacket(..) => Ok(HandlePacket::Disconnect),

                    VariablePacket::PubackPacket(ref puback) => {
                        debug!("*** puback --> {:?}\n @@@ queue --> {:?}\n\n", puback, self.outgoing_pub);
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
                                error!("Oopssss..unsolicited ack --> {:?}\n", puback);
                                None
                            }
                        };
                        debug!("Pub Q Len After Ack @@@ {:?}", self.outgoing_pub.len());
                        Ok(HandlePacket::PubAck(m))
                    }

                    VariablePacket::PublishPacket(ref publ) => {
                        let message = try!(Message::from_pub(publ));
                        self._handle_message(message)
                    }

                    // @ Qos2 message published by client is recorded by broker
                    // @ Remove message from 'outgoing_rec' queue and add pkid to 'outgoing_rel'
                    // @ Send 'pubrel' to broker
                    VariablePacket::PubrecPacket(ref pubrec) => {
                        let pkid = pubrec.packet_identifier();
                        let m = match self.outgoing_rec
                            .iter()
                            .position(|ref x| x.1.get_pkid() == Some(pkid)) {
                            Some(i) => {
                                if let Some(m) = self.outgoing_rec.remove(i) {
                                    Some(*m.1)
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
                        self.outgoing_rel.push_back((time::get_time().sec, PacketIdentifier(pkid)));
                        // NOTE: Don't Error return here. It's ok to fail during writes coz of disconnection.
                        // `force_transmit` will resend when reconnection is successful
                        let _ = self._pubrel(pkid);
                        Ok(HandlePacket::PubRec(m))
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
                                error!("Oopssss..unsolicited release. Message might have already been released --> {:?}", pubrel);
                                None
                            }
                        };

                        self.outgoing_comp.push_back((time::get_time().sec, PacketIdentifier(pkid)));
                        let _ = self._pubcomp(pkid);

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
                Ok(HandlePacket::None)
            }
        }
    }

    fn _try_retransmit(&mut self) -> Result<()> {
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

        Ok(())
    }

    // Spec says that client (for QoS > 0, clean session) should retransmit all the
    // unacked messages after reconnection. Instead of waiting for retransmit
    // timeout
    // to kickin, this methods retransmits everthing in the queue immediately
    // NOTE: outgoing_rels are handled in _try_retransmit. Sending duplicate pubrels
    // isn't a problem (I guess ?). Broker will just resend pubcomps
    fn _force_retransmit(&mut self) {
        if self.opts.clean_session {
            // Cloning because iterating and removing isn't possible.
            // Iterating over indexes and and removing elements messes
            // up the remove sequence
            let mut outgoing_pub = self.outgoing_pub.clone();
            self.outgoing_pub.clear();
            while let Some(message) = outgoing_pub.pop_front() {
                let _ = self._publish(*message.1);
            }

            let mut outgoing_rec = self.outgoing_rec.clone();
            self.outgoing_rec.clear();
            while let Some(message) = outgoing_rec.pop_front() {
                let _ = self._publish(*message.1);
            }

            let mut outgoing_rel = self.outgoing_rel.clone();
            self.outgoing_rel.clear();
            while let Some(rel) = outgoing_rel.pop_front() {
                self.outgoing_rel.push_back((time::get_time().sec, rel.1));
                let PacketIdentifier(pkid) = rel.1;
                let _ = self._pubrel(pkid);
            }

            let mut outgoing_comp = self.outgoing_comp.clone();
            self.outgoing_comp.clear();
            while let Some(comp) = outgoing_comp.pop_front() {
                self.outgoing_comp.push_back((time::get_time().sec, comp.1));
                let PacketIdentifier(pkid) = comp.1;
                let _ = self._pubcomp(pkid);
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
        Ok(())
    }

    pub fn _disconnect(&mut self) -> Result<()> {
        let disconnect = try!(genpack::generate_disconnect_packet());
        try!(self._write_packet(disconnect));
        Ok(())
    }

    fn _subscribe(&mut self, topics: Vec<(TopicFilter, QualityOfService)>) -> Result<()> {
        let subscribe_packet = try!(genpack::generate_subscribe_packet(topics));
        try!(self._write_packet(subscribe_packet));
        Ok(())
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
            }
            QoSWithPacketIdentifier::Level2(_) => {
                self.outgoing_rec.push_back((time::get_time().sec, message_box.clone()));
            }
        }
        // error!("Queue --> {:?}\n\n", self.outgoing_pub);
        // debug!("       Publish {:?} {:?} > {} bytes", message.qos,
        // topic.clone().to_string(), message.payload.len());


        match message.qos {
            QoSWithPacketIdentifier::Level0 => try!(self._write_packet(publish_packet)),
            QoSWithPacketIdentifier::Level1(_) |
            QoSWithPacketIdentifier::Level2(_) => {
                if self.state == MqttState::Connected {
                    try!(self._write_packet(publish_packet));
                } else {
                    error!("State = {:?}. Skip network write", self.state);
                }
            }
        };
        Ok(())
    }

    fn _ping(&mut self) -> Result<()> {
        let ping = try!(genpack::generate_pingreq_packet());
        // self.await_ping = true;
        try!(self._write_packet(ping));
        self._flush()
    }

    fn _puback(&mut self, pkid: u16) -> Result<()> {
        let puback_packet = try!(genpack::generate_puback_packet(pkid));
        try!(self._write_packet(puback_packet));
        Ok(())
    }

    fn _pubrec(&mut self, pkid: u16) -> Result<()> {
        let pubrec_packet = try!(genpack::generate_pubrec_packet(pkid));
        try!(self._write_packet(pubrec_packet));
        Ok(())
    }

    fn _pubrel(&mut self, pkid: u16) -> Result<()> {
        let pubrel_packet = try!(genpack::generate_pubrel_packet(pkid));
        try!(self._write_packet(pubrel_packet));
        Ok(())
    }

    fn _pubcomp(&mut self, pkid: u16) -> Result<()> {
        let puback_packet = try!(genpack::generate_pubcomp_packet(pkid));
        try!(self._write_packet(puback_packet));
        Ok(())
    }

    fn _unbind(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
        self.state = MqttState::Disconnected;
        info!("  Disconnected {:?}", self.opts.client_id);
    }

    #[inline]
    fn _write_packet(&mut self, packet: Vec<u8>) -> Result<()> {
        try!(self.stream.write_all(&packet));
        try!(self._flush());
        Ok(())
    }

    fn _flush(&mut self) -> Result<()> {
        try!(self.stream.flush());
        self.last_flush = Instant::now();
        Ok(())
    }
}
