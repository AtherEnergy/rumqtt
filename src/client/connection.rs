use std::sync::mpsc as stdmpsc;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use std::collections::VecDeque;
use std::io::{self, ErrorKind};
use std::error::Error;
use std::thread;

use futures::prelude::*;
use futures::stream::{Stream, SplitSink};
use futures::sync::mpsc::{self, Sender, Receiver};
use tokio_core::reactor::Core;
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
    addr: SocketAddr,
    opts: MqttOptions,

    // --------  State  ----------
    connection_state: MqttConnectionState,
    initial_connect: bool,
    await_pingresp: bool,
    last_flush: Instant,
    last_pkid: PacketIdentifier,

    // For QoS 1. Stores outgoing publishes
    outgoing_pub: VecDeque<Message>,
    // clean_session=false will remember subscriptions only till lives.
    // If broker crashes, all its state will be lost (most brokers).
    // client wouldn't want to loose messages after it comes back up again
    subscriptions: VecDeque<SubscribeTopic>,

    // --------  Callbacks  --------
    // callback: Option<MqttCallback>,

    // -------- Thread pool to execute callbacks
    pool: ThreadPool,
}

#[derive(Debug)]
pub enum NetworkRequest {
    Subscribe(Vec<(TopicPath, QoS)>),
    Publish(Publish),
    Ping,
}

pub fn start(opts: MqttOptions, tx_commands_tx: stdmpsc::SyncSender<Sender<NetworkRequest>>) {
    loop {
        let mut reactor = Core::new().unwrap();
        let handle = reactor.handle();
        // TODO: fix the clone
        let opts = opts.clone();
        let tx_commands_tx = tx_commands_tx.clone();
        // config
        // NOTE: make sure that dns resolution happens during reconnection incase 
        //       ip of the server changes
        // TODO: Handle all the unwraps here
        let addr: SocketAddr = opts.broker_addr.as_str().parse().unwrap();
        let id = opts.client_id;
        let keep_alive = opts.keep_alive.unwrap();
        let clean_session =  opts.clean_session;
        let reconnect_after = opts.reconnect_after.unwrap();

        let client = async_block! {
            let stream = await!(TcpStream::connect(&addr, &handle))?;
            let connect = packet::gen_connect_packet(&id, keep_alive, clean_session, None, None);
            let framed = stream.framed(MqttCodec);
            let framed = await!(framed.send(connect)).unwrap();

            // create a 'tx' and send it to client after every succesful connection
            // so that it can sent network requests to this 'connection' thread
            let (commands_tx, commands_rx) = mpsc::channel::<NetworkRequest>(10);
            let ping_commands_tx = commands_tx.clone();
            tx_commands_tx.try_send(commands_tx).unwrap();


            let (sender, receiver) = framed.split();

            // ping timer
            handle.spawn(ping_timer(ping_commands_tx, keep_alive).then(|result| {
                match result {
                    Ok(_) => println!("Ping timer done"),
                    Err(e) => println!("Ping timer IO error {:?}", e),
                }
                Ok(())
            }));

            // network transmission requests
            handle.spawn(command_read(commands_rx, sender).then(|result| {
                match result {
                    Ok(_) => println!("Command receiver done"),
                    Err(e) => println!("Command IO error {:?}", e),
                }
                Ok(())
            }));

            // incoming network messages
            #[async]
            for msg in receiver {
                println!("message = {:?}", msg);
            }

            error!("Done with network receiver !!");
            Ok::<(), io::Error>(())
        }; // async client

        let response = reactor.run(client);
        println!("{:?}", response);
        thread::sleep(Duration::new(reconnect_after as u64, 0));
    }
}

#[async]
fn ping_timer(mut commands_tx: Sender<NetworkRequest>, keep_alive: u16) -> io::Result<()> {
    let timer = Timer::default();
    let interval = timer.interval(Duration::new(keep_alive as u64, 0));

    #[async]
    for _t in interval {
        println!("Ping timer fire");
        commands_tx = await!(
            commands_tx.send(NetworkRequest::Ping).or_else(|e| {
                Err(io::Error::new(ErrorKind::Other, e.description()))
            })
        )?;
    }

    Ok(())
}

#[async]
fn command_read(commands_rx: Receiver<NetworkRequest>, mut sender: SplitSink<Framed<TcpStream, MqttCodec>>) -> io::Result<()> {

    let commands_rx = commands_rx.or_else(|_| {
            Err(io::Error::new(ErrorKind::Other, "Rx Error"))
    });
    
    #[async]
    for command in commands_rx {
        println!("command = {:?}", command);
        let packet = match command {
            NetworkRequest::Publish(publish) => {
                Packet::Publish(publish)
            }
            NetworkRequest::Ping => {
                packet::gen_pingreq_packet()
            }
            _ => unimplemented!()
        };

        sender = await!(sender.send(packet))?
    }

    Ok(())
}

#[cfg(test)]
mod test {
}