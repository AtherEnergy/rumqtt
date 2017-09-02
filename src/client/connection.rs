use std::sync::mpsc as stdmpsc;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use std::collections::VecDeque;
use std::io::{self, ErrorKind};
use std::error::Error;
use std::thread;

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
    Drain,
}


pub fn start(opts: MqttOptions, commands_tx: Sender<NetworkRequest>, commands_rx: Receiver<NetworkRequest>) {
    let mut reactor = Core::new().unwrap();
    let mut commands_rx = commands_rx.or_else(|_| {
        Err(io::Error::new(ErrorKind::Other, "Rx Error"))
    }).boxed();

    loop {
        let handle = reactor.handle();
        let commands_tx = commands_tx.clone();
        // TODO: fix the clone
        let opts = opts.clone();
        // config
        // NOTE: make sure that dns resolution happens during reconnection incase  ip of the server changes
        // TODO: Handle all the unwraps here
        let addr: SocketAddr = opts.broker_addr.as_str().parse().unwrap();
        let id = opts.client_id;
        let keep_alive = opts.keep_alive.unwrap();
        let clean_session =  opts.clean_session;
        let reconnect_after = opts.reconnect_after.unwrap();

        let client = async_block! {
            let connect = packet::gen_connect_packet(&id, keep_alive, clean_session, None, None);
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
            handle.spawn(mqtt_recv(receiver, nw_commands_tx).then(|result| {
                match result {
                    Ok(_) => println!("N/w receiver done"),
                    Err(e) => println!("N/w IO error {:?}", e),
                }
                Ok(())
            }));

            // ping timer
            handle.spawn(ping_timer(ping_commands_tx, keep_alive).then(|result| {
                match result {
                    Ok(_) => println!("Ping timer done"),
                    Err(e) => println!("Ping timer IO error {:?}", e),
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

                println!("command = {:?}", command);
                let packet = match command {
                    NetworkRequest::Publish(publish) => Packet::Publish(publish),
                    NetworkRequest::Ping => Packet::Pingreq,
                    NetworkRequest::Drain => break,
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
fn ping_timer(mut commands_tx: Sender<NetworkRequest>, keep_alive: u16) -> io::Result<()> {
    let timer = Timer::default();
    let interval = timer.interval(Duration::new(keep_alive as u64, 0));

    #[async]
    for _t in interval {
        debug!("Ping timer fire");
        commands_tx = await!(
            commands_tx.send(NetworkRequest::Ping).or_else(|e| {
                Err(io::Error::new(ErrorKind::Other, e.description()))
            })
        )?;
    }

    Ok(())
}

#[async]
fn mqtt_recv(receiver: SplitStream<Framed<TcpStream, MqttCodec>>, commands_tx: Sender<NetworkRequest>) -> io::Result<()> {
    
    #[async]
    for message in receiver {
        println!("incoming n/w message = {:?}", message);
    }

    await!(commands_tx.send(NetworkRequest::Drain));
    Ok(())
}

#[cfg(test)]
mod test {
}