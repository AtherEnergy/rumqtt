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
    fn handle_outgoing_publish(&mut self, mut publish: Publish) -> Packet {
        match publish.qos {
            QoS::AtMostOnce => Packet::Publish(publish),
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
                Packet::Publish(publish)
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

        // config
        // NOTE: make sure that dns resolution happens during reconnection incase  ip of the server changes
        // TODO: Handle all the unwraps here
        let addr: SocketAddr = opts.broker_addr.as_str().parse().unwrap();
        let reconnect_after = opts.reconnect_after.unwrap();

        let client = async_block! {
            let connect = packet::gen_connect_packet(&opts.client_id, opts.keep_alive.unwrap(), opts.clean_session, None, None);
            let framed = match await!(mqtt_connect(addr, handle.clone(), connect)) {
                Ok(s) => s,
                Err(e) => {
                    error!("Connection error = {:?}", e);
                    return Ok(commands_rx);
                }
            };

            let (mut sender, receiver) = framed.split();

            let ping_commands_tx = commands_tx.clone();
            let nw_commands_tx = commands_tx.clone();
            
            // incoming network messages
            handle.spawn(mqtt_recv(mqtt_state_mqtt_recv, receiver, nw_commands_tx).then(|result| {
                match result {
                    Ok(_) => error!("N/w receiver done"),
                    Err(e) => error!("N/w IO error {:?}", e),
                }
                Ok(())
            }));

            // ping timer
            handle.spawn(ping_timer(ping_commands_tx, opts.keep_alive.unwrap()).then(|result| {
                match result {
                    Ok(_) => error!("Ping timer done"),
                    Err(e) => error!("Ping timer IO error {:?}", e),
                }
                Ok(())
            }));
                
            // execute user requests  
            loop {
                let command = match await!(commands_rx.into_future().map_err(|e| e.0))? {
                    (Some(item), s) => {
                        commands_rx = s;
                        item
                    }
                    (None, s) => {
                        commands_rx = s;
                        break

                    }
                };

                info!("command = {:?}", command);

                let packet = match command {
                    Request::Publish(publish) => mqtt_state.borrow_mut().handle_outgoing_publish(publish),
                    Request::Ping => Packet::Pingreq,
                    Request::Reconnect => break,
                    _ => unimplemented!()
                };

                sender = await!(sender.send(packet))?
            }

            error!("Done with network receiver !!");
            Ok::<_, io::Error>(commands_rx)
        }; // async client


        let response = reactor.run(client);
        commands_rx = response.unwrap();
        
        info!("Will retry connection again in {} seconds", reconnect_after);
        thread::sleep(Duration::new(reconnect_after as u64, 0));
    }
}

#[async]
fn mqtt_connect(addr: SocketAddr, handle: Handle, connect: Connect) -> io::Result<Framed<TcpStream, MqttCodec>> {
    let stream = await!(TcpStream::connect(&addr, &handle))?;
    let connect = Packet::Connect(connect);
    
    let framed = stream.framed(MqttCodec);
    let framed = await!(framed.send(connect))?;
    Ok(framed)
}

#[async]
fn ping_timer(mut commands_tx: Sender<Request>, keep_alive: u16) -> io::Result<()> {
    let timer = Timer::default();
    let interval = timer.interval(Duration::new(keep_alive as u64, 0));

    #[async]
    for _t in interval {
        debug!("Ping timer fire");
        commands_tx = await!(
            commands_tx.send(Request::Ping).or_else(|e| {
                Err(io::Error::new(ErrorKind::Other, e.description()))
            })
        )?;
    }

    Ok(())
}

#[async]
fn mqtt_recv(mqtt_state: Rc<RefCell<MqttState>>, receiver: SplitStream<Framed<TcpStream, MqttCodec>>, commands_tx: Sender<Request>) -> io::Result<()> {
    
    #[async]
    for message in receiver {
        info!("incoming n/w message = {:?}", message);
    }

    error!("Network reciever stopped. Sending reconnect request");
    await!(commands_tx.send(Request::Reconnect));
    Ok(())
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

        let packet = mqtt.handle_outgoing_publish(publish);
        if let Packet::Publish(publish_out) = packet {
            // pkid shouldn't be added
            assert_eq!(publish_out.pid, None);
            // publish shouldn't added to queue
            assert_eq!(mqtt.outgoing_pub.len(), 0);
        } else {
            panic!("Should have been a publish packet");
        }

        // QoS1 Publish
        let publish = Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        let packet = mqtt.handle_outgoing_publish(publish.clone());

        if let Packet::Publish(publish_out) = packet {
            // pkid shouldn't be added
            assert_eq!(publish_out.pid, Some(PacketIdentifier(1)));
            // publish shouldn't added to queue
            assert_eq!(mqtt.outgoing_pub.len(), 1);
        } else {
            panic!("Should have been a publish packet");
        }

        let packet = mqtt.handle_outgoing_publish(publish.clone());
        if let Packet::Publish(publish_out) = packet {
            // pkid shouldn't be added
            assert_eq!(publish_out.pid, Some(PacketIdentifier(2)));
            // publish shouldn't added to queue
            assert_eq!(mqtt.outgoing_pub.len(), 2);
        } else {
            panic!("Should have been a publish packet");
        }
    }
}