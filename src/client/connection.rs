use std::net::SocketAddr;
use std::time::{Duration, Instant};
use std::collections::VecDeque;
use std::io::{self, ErrorKind};
use std::error::Error;
use std::thread;
use std::cell::RefCell;
use std::rc::Rc;

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
use packet;
use MqttOptions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MqttConnectionState {
    Handshake,
    Connected,
    Disconnected,
}

struct MqttState {
    opts: MqttOptions,

    // --------  State  ----------
    connection_state: MqttConnectionState,
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

impl MqttState {
    fn new(opts: MqttOptions) -> Self {
        MqttState {
            opts: opts,
            connection_state: MqttConnectionState::Disconnected,
            initial_connect: true,
            await_pingresp: false,
            last_flush: Instant::now(),
            last_pkid: PacketIdentifier(0),
            outgoing_pub: VecDeque::new(),
            subscriptions: VecDeque::new(),
            pool: ThreadPool::new(1),
        }
    }

    /// Sets next packet id if pkid is None (fresh publish) and adds it to the
    /// outgoing publish queue
    fn handle_outgoing_publish(&mut self, mut publish: Publish) -> Publish {
        match publish.qos {
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
        }
    }

    pub fn handle_incoming_puback(&mut self, pkid: PacketIdentifier) -> Option<Publish> {
        if let Some(index) = self.outgoing_pub.iter().position(|x| x.pid == Some(pkid)) {
            self.outgoing_pub.remove(index)
        } else {
            error!("Unsolicited PUBLISH packet: {:?}", pkid);
            None
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
    Subscribe(Vec<(TopicPath, QoS)>),
    Publish(Publish),
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

    use super::MqttState;
    use mqtt3::*;
    use mqttopts::MqttOptions;

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
        assert_eq!(publish_out.pid, None);
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
        assert_eq!(publish_out.pid, Some(PacketIdentifier(1)));
        // publish shouldn't added to queue
        assert_eq!(mqtt.outgoing_pub.len(), 1);

        let publish_out = mqtt.handle_outgoing_publish(publish.clone());
        // pkid shouldn't be added
        assert_eq!(publish_out.pid, Some(PacketIdentifier(2)));
        // publish shouldn't added to queue
        assert_eq!(mqtt.outgoing_pub.len(), 2);
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
}