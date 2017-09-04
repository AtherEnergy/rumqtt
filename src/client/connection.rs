use std::net::SocketAddr;
use std::time::{Duration, Instant};
use std::collections::VecDeque;
use std::io::{self, ErrorKind};
use std::thread;
use std::cell::RefCell;
use std::rc::Rc;
use std::result::Result;
use std::mem;

use futures::prelude::*;
use futures::stream::{Stream, SplitSink, SplitStream};
use futures::sync::mpsc::{self, Sender, Receiver};
use tokio_core::reactor::{Core, Handle};
use tokio_core::net::TcpStream;
use tokio_timer::Timer;
use tokio_io::AsyncRead;
use tokio_io::codec::Framed;
use mqtt3::*;
use threadpool::ThreadPool;

use codec::MqttCodec;
use error::{PingError, ConnectError, PublishError, PubackError, SubscribeError, SubackError};
use packet;
use MqttOptions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MqttConnectionStatus {
    Handshake,
    Connected,
    Disconnected,
}

struct MqttState {
    opts: MqttOptions,

    // --------  State  ----------
    connection_status: MqttConnectionStatus,
    initial_connect: bool,
    await_pingresp: bool,
    last_flush: Instant,
    last_pkid: PacketIdentifier,

    // For QoS 1. Stores outgoing publishes
    outgoing_pub: VecDeque<Publish>,
    // clean_session=false will remember subscriptions only till lives.
    // If broker crashes, all its state will be lost (most brokers).
    // client wouldn't want to loose messages after it comes back up again
    subscriptions: VecDeque<SubscribeTopic>,

    // --------  Callbacks  --------
    // callback: Option<MqttCallback>,

    // -------- Thread pool to execute callbacks
    pool: ThreadPool,
}

/// Design: MqttState methods will just modify the state of the object
///         but doesn't do any network operations. Methods will do
///         appropriate returns so that n/w methods or n/w eventloop can
///         operate directly. This abstracts the functionality better
///         so that it's easy to switch between synchronous code, tokio (or)
///         async/await

impl MqttState {
    fn new(opts: MqttOptions) -> Self {
        MqttState {
            opts: opts,
            connection_status: MqttConnectionStatus::Disconnected,
            initial_connect: true,
            await_pingresp: false,
            last_flush: Instant::now(),
            last_pkid: PacketIdentifier(0),
            outgoing_pub: VecDeque::new(),
            subscriptions: VecDeque::new(),
            pool: ThreadPool::new(1),
        }
    }

    fn handle_outgoing_connect(&mut self) -> Connect {
        let keep_alive = if let Some(keep_alive) = self.opts.keep_alive {
            keep_alive
        } else {
            180
        };

        self.connection_status = MqttConnectionStatus::Handshake;

        let (username, password) = if let Some((ref username, ref password)) = self.opts.credentials {
            (Some(username.to_owned()), Some(password.to_owned()))
        } else {
            (None, None)
        };

        packet::gen_connect_packet(&self.opts.client_id, keep_alive, self.opts.clean_session, username, password)
    }

    fn handle_incoming_connack(&mut self, connack: Connack) -> Result<Option<VecDeque<Publish>>, ConnectError> {
        let response = connack.code;
        if response != ConnectReturnCode::Accepted {
            self.connection_status = MqttConnectionStatus::Disconnected;
            Err(response)?
        } else {
            self.connection_status = MqttConnectionStatus::Connected;
            let publishes = mem::replace(&mut self.outgoing_pub, VecDeque::new());

            if self.opts.clean_session {
                Ok(None)
            } else {
                Ok(Some(publishes))
            }
        }
    }

    /// Sets next packet id if pkid is None (fresh publish) and adds it to the
    /// outgoing publish queue
    fn handle_outgoing_publish(&mut self, mut publish: Publish) -> Result<Publish, PublishError> {
        let payload_len = publish.payload.len();

        if payload_len > self.opts.max_packet_size {
            error!("Size limit exceeded. Dropping packet: {:?}", publish);
            return Err(PublishError::PacketSizeLimitExceeded)
        }

        let publish = match publish.qos {
            QoS::AtMostOnce => publish,
            QoS::AtLeastOnce => {
                // add pkid if None
                let publish = if publish.pid == None {
                    let pkid = self.next_pkid();
                    publish.pid = Some(pkid);
                    publish
                } else {
                    publish
                };

                self.outgoing_pub.push_back(publish.clone());
                publish
            }
            _ => unimplemented!()
        };

        if self.connection_status == MqttConnectionStatus::Connected {
            Ok(publish)
        } else {
            Err(PublishError::InvalidState)
        }

    }

    pub fn handle_incoming_puback(&mut self, pkid: PacketIdentifier) -> Result<Publish, PubackError> {
        if let Some(index) = self.outgoing_pub.iter().position(|x| x.pid == Some(pkid)) {
            Ok(self.outgoing_pub.remove(index).unwrap())
        } else {
            error!("Unsolicited PUBLISH packet: {:?}", pkid);
            Err(PubackError::Unsolicited)
        }
    }

    // check when the last control packet/pingreq packet
    // is received and return the status which tells if
    // keep alive time has exceeded
    // NOTE: status will be checked for zero keepalive times also
    pub fn handle_outgoing_ping(&mut self) -> Result<bool, PingError> {
        if let Some(keep_alive) = self.opts.keep_alive {
            let elapsed = self.last_flush.elapsed();

            if elapsed >= Duration::from_millis(((keep_alive * 1000) as f64 * 0.9) as u64) {
                if elapsed >= Duration::new((keep_alive + 1) as u64, 0) {
                    return Err(PingError::Timeout);
                }
                // @ Prevents half open connections. Tcp writes will buffer up
                // with out throwing any error (till a timeout) when internet
                // is down. Eventhough broker closes the socket after timeout,
                // EOF will be known only after reconnection.
                // We need to unbind the socket if there in no pingresp before next ping
                // (What about case when pings aren't sent because of constant publishes
                // ?. A. Tcp write buffer gets filled up and write will be blocked for 10
                // secs and then error out because of timeout.)
                if self.await_pingresp {
                    return Err(PingError::AwaitPingResp);
                }

                if self.connection_status == MqttConnectionStatus::Connected {
                    self.last_flush = Instant::now();
                    self.await_pingresp = true;
                    return Ok(true)
                } else {
                    error!("State = {:?}. Shouldn't ping in this state", self.connection_status);
                    return Err(PingError::InvalidState)
                }
            }
        }
        // no need to ping
        Ok(false)
    }

    pub fn handle_incoming_pingresp(&mut self) {
        self.await_pingresp = false;
    }

    pub fn handle_outgoing_subscribe(&mut self, topics: Vec<SubscribeTopic>) -> Result<Subscribe, SubscribeError> {
        let pkid = self.next_pkid();

        Ok(Subscribe {
            pid: pkid,
            topics: topics,
        })
    }


    pub fn handle_incoming_suback(&mut self, ack: Suback) -> Result<(), SubackError> {
        if ack.return_codes.iter().any(|v| *v == SubscribeReturnCodes::Failure) {
            Err(SubackError::Rejected)
        } else {
            Ok(())
        }
    }

    pub fn handle_disconnect(&mut self) {
        self.await_pingresp = false;
        self.connection_status = MqttConnectionStatus::Disconnected;

        // remove all the state
        if self.opts.clean_session {
            self.outgoing_pub.clear();
        }
    }

    // http://stackoverflow.com/questions/11115364/mqtt-messageid-practical-implementation
    fn next_pkid(&mut self) -> PacketIdentifier {
        let PacketIdentifier(mut pkid) = self.last_pkid;
        if pkid == 65535 {
            pkid = 0;
        }
        self.last_pkid = PacketIdentifier(pkid + 1);
        self.last_pkid
    }
}

#[derive(Debug)]
pub enum Request {
    Subscribe(Vec<SubscribeTopic>),
    Publish(Publish),
    Connect,
    Ping,
    Reconnect,
}


pub fn start(opts: MqttOptions, commands_tx: Sender<Request>, commands_rx: Receiver<Request>) {

    let mut commands_rx = commands_rx.or_else(|_| {
        Err(io::Error::new(ErrorKind::Other, "Rx Error"))
    });

    loop {
        // NOTE: If we move this out, what happen's to futures spawned in previous iteration? memory keeps growing?
        let mut reactor = Core::new().unwrap();
        let handle = reactor.handle();
        let commands_tx = commands_tx.clone();
        // TODO: fix the clone
        let opts = opts.clone();

        let mqtt_state = Rc::new(RefCell::new(MqttState::new(opts.clone())));
        let mqtt_state_connect = mqtt_state.clone();
        let mqtt_state_mqtt_recv = mqtt_state.clone();
        let mqtt_state_ping = mqtt_state.clone();

        // config
        // NOTE: make sure that dns resolution happens during reconnection incase  ip of the server changes
        // TODO: Handle all the unwraps here
        let addr: SocketAddr = opts.broker_addr.as_str().parse().unwrap();
        let reconnect_after = opts.reconnect_after.unwrap();

        
        info!("Will retry connection again in {} seconds", reconnect_after);
        thread::sleep(Duration::new(reconnect_after as u64, 0));
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use super::{MqttState, MqttConnectionStatus};
    use mqtt3::*;
    use mqttopts::MqttOptions;
    use error::{PingError, PublishError};

    #[test]
    fn next_pkid_roll() {
        let mut mqtt = MqttState::new(MqttOptions::new("test-id", "127.0.0.1:1883"));
        let mut pkt_id = PacketIdentifier(0);
        for _ in 0..65536 {
            pkt_id = mqtt.next_pkid();
        }
        assert_eq!(PacketIdentifier(1), pkt_id);
    }

    #[test]
    fn outgoing_publish_handle_should_set_pkid_correctly_and_add_publish_to_queue_correctly() {
        let mut mqtt = MqttState::new(MqttOptions::new("test-id", "127.0.0.1:1883"));
        mqtt.connection_status = MqttConnectionStatus::Connected;

        // QoS0 Publish
        let publish = Publish {
            dup: false,
            qos: QoS::AtMostOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        let publish_out = mqtt.handle_outgoing_publish(publish);
        // pkid shouldn't be added
        assert_eq!(publish_out.unwrap().pid, None);
        // publish shouldn't added to queue
        assert_eq!(mqtt.outgoing_pub.len(), 0);
        

        // QoS1 Publish
        let publish = Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        let publish_out = mqtt.handle_outgoing_publish(publish.clone());
        // pkid shouldn't be added
        assert_eq!(publish_out.unwrap().pid, Some(PacketIdentifier(1)));
        // publish shouldn't added to queue
        assert_eq!(mqtt.outgoing_pub.len(), 1);

        let publish_out = mqtt.handle_outgoing_publish(publish.clone());
        // pkid shouldn't be added
        assert_eq!(publish_out.unwrap().pid, Some(PacketIdentifier(2)));
        // publish shouldn't added to queue
        assert_eq!(mqtt.outgoing_pub.len(), 2);
    }

    #[test]
    fn outgoing_publish_handle_should_throw_error_in_invalid_state() {
        let mut mqtt = MqttState::new(MqttOptions::new("test-id", "127.0.0.1:1883"));

        let publish = Publish {
            dup: false,
            qos: QoS::AtMostOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        let publish_out = mqtt.handle_outgoing_publish(publish);
        assert_eq!(publish_out, Err(PublishError::InvalidState));
    }

    #[test]
    fn outgoing_publish_handle_should_throw_error_when_packetsize_exceeds_max() {
        let mut mqtt = MqttState::new(MqttOptions::new("test-id", "127.0.0.1:1883"));

        let publish = Publish {
            dup: false,
            qos: QoS::AtMostOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![0; 101 * 1024]),
        };

        let publish_out = mqtt.handle_outgoing_publish(publish);
        assert_eq!(publish_out, Err(PublishError::PacketSizeLimitExceeded));
    }

    #[test]
    fn incoming_puback_should_remove_correct_publish_from_queue() {
        let mut mqtt = MqttState::new(MqttOptions::new("test-id", "127.0.0.1:1883"));
        // QoS1 Publish
        let publish = Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        let publish_out = mqtt.handle_outgoing_publish(publish.clone());
        let publish_out = mqtt.handle_outgoing_publish(publish.clone());
        let publish_out = mqtt.handle_outgoing_publish(publish);

        let publish = mqtt.handle_incoming_puback(PacketIdentifier(1)).unwrap();
        assert_eq!(publish.pid, Some(PacketIdentifier(1)));
        assert_eq!(mqtt.outgoing_pub.len(), 2);

        let publish = mqtt.handle_incoming_puback(PacketIdentifier(2)).unwrap();
        assert_eq!(publish.pid, Some(PacketIdentifier(2)));
        assert_eq!(mqtt.outgoing_pub.len(), 1);

        let publish = mqtt.handle_incoming_puback(PacketIdentifier(3)).unwrap();
        assert_eq!(publish.pid, Some(PacketIdentifier(3)));
        assert_eq!(mqtt.outgoing_pub.len(), 0);
    }

    #[test]
    fn outgoing_ping_handle_should_throw_errors_during_invalid_state() {
        // 1. test for invalid state
        let mut mqtt = MqttState::new(MqttOptions::new("test-id", "127.0.0.1:1883"));
        mqtt.opts.keep_alive = Some(5);
        // first ping always returns success but with ping false
        assert_eq!(Ok(false), mqtt.handle_outgoing_ping());
        thread::sleep(Duration::new(5, 0));
        assert_eq!(Err(PingError::InvalidState), mqtt.handle_outgoing_ping());
    }

    #[test]
    fn outgoing_ping_handle_should_throw_errors_for_no_pingresp() {
        let mut mqtt = MqttState::new(MqttOptions::new("test-id", "127.0.0.1:1883"));
        mqtt.opts.keep_alive = Some(5);
        mqtt.connection_status = MqttConnectionStatus::Connected;
        thread::sleep(Duration::new(5, 0));
        // should ping
        assert_eq!(Ok(true), mqtt.handle_outgoing_ping());
        thread::sleep(Duration::new(5, 0));
        // should throw error because we didn't get pingresp for previous ping
        assert_eq!(Err(PingError::AwaitPingResp), mqtt.handle_outgoing_ping());
    }

    #[test]
    fn outgoing_ping_handle_should_throw_error_if_ping_time_exceeded() {
        let mut mqtt = MqttState::new(MqttOptions::new("test-id", "127.0.0.1:1883"));
        mqtt.opts.keep_alive = Some(5);
        mqtt.connection_status = MqttConnectionStatus::Connected;
        thread::sleep(Duration::new(7, 0));
        // should ping
        assert_eq!(Err(PingError::Timeout), mqtt.handle_outgoing_ping());
    }

    #[test]
    fn outgoing_ping_handle_should_succeed_if_pingresp_is_received() {
        let mut mqtt = MqttState::new(MqttOptions::new("test-id", "127.0.0.1:1883"));
        mqtt.opts.keep_alive = Some(5);
        mqtt.connection_status = MqttConnectionStatus::Connected;
        thread::sleep(Duration::new(5, 0));
        // should ping
        assert_eq!(Ok(true), mqtt.handle_outgoing_ping());
        mqtt.handle_incoming_pingresp();
        thread::sleep(Duration::new(5, 0));
        // should ping
        assert_eq!(Ok(true), mqtt.handle_outgoing_ping());
    }

    #[test]
    fn disconnect_handle_should_reset_everything_in_clean_session() {
        let mut mqtt = MqttState::new(MqttOptions::new("test-id", "127.0.0.1:1883"));
        mqtt.await_pingresp = true;
        // QoS1 Publish
        let publish = Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish);

        mqtt.handle_disconnect();
        assert_eq!(mqtt.outgoing_pub.len(), 0);
        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Disconnected);
        assert_eq!(mqtt.await_pingresp, false);
    }

    #[test]
    fn disconnect_handle_should_reset_everything_except_queues_in_persistent_session() {
        let mut mqtt = MqttState::new(MqttOptions::new("test-id", "127.0.0.1:1883"));
        mqtt.await_pingresp = true;
        mqtt.opts.clean_session = false;
        // QoS1 Publish
        let publish = Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish);

        mqtt.handle_disconnect();
        assert_eq!(mqtt.outgoing_pub.len(), 3);
        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Disconnected);
        assert_eq!(mqtt.await_pingresp, false);
    }

    #[test]
    fn connection_status_is_valid_while_handling_connect_and_connack_packets() {
        let mut mqtt = MqttState::new(MqttOptions::new("test-id", "127.0.0.1:1883"));

        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Disconnected);
        mqtt.handle_outgoing_connect();
        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Handshake);

        let connack = Connack {
            session_present: false,
            code: ConnectReturnCode::Accepted
        };

        mqtt.handle_incoming_connack(connack);
        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Connected);

        let connack = Connack {
            session_present: false,
            code: ConnectReturnCode::BadUsernamePassword
        };

        mqtt.handle_incoming_connack(connack);
        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Disconnected);
    }

    #[test]
    fn connack_handle_should_not_return_list_of_incomplete_messages_to_be_sent_in_persistent_session() {
        let mut mqtt = MqttState::new(MqttOptions::new("test-id", "127.0.0.1:1883"));

        let publish = Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish);

        let connack = Connack {
            session_present: false,
            code: ConnectReturnCode::Accepted
        };

        if let Ok(p) = mqtt.handle_incoming_connack(connack) {
            assert_eq!(None, p);
        }
    }

    #[test]
    fn connack_handle_should_return_list_of_incomplete_messages_to_be_sent_in_persistent_session() {
        let mut mqtt = MqttState::new(MqttOptions::new("test-id", "127.0.0.1:1883"));
        mqtt.opts.clean_session = false;

        let publish = Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish);

        let connack = Connack {
            session_present: false,
            code: ConnectReturnCode::Accepted
        };

        if let Ok(v) = mqtt.handle_incoming_connack(connack) {
            if let Some(v) = v {
                assert_eq!(v.len(), 3);
            } else {
                panic!("Should return list of publishes");
            }
        }

        assert_eq!(0, mqtt.outgoing_pub.len());
    }
}