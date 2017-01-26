extern crate tokio_mqtt;
extern crate mqtt3;
extern crate futures;
extern crate tokio_core as core;

use futures::Future;
use futures::Sink;
use futures::Stream;
use core::net::TcpStream;
use core::io::Io;

use tokio_mqtt::codec::MqttCodec;
use tokio_mqtt::packet::*;
use tokio_mqtt::clientoptions::MqttOptions;
use tokio_mqtt::connection::Connection;

use std::net::{SocketAddr, ToSocketAddrs};
use std::thread;
use std::time::Duration;

fn lookup_ipv4<A: ToSocketAddrs>(addr: A) -> SocketAddr {
    let addrs = addr.to_socket_addrs().expect("Conversion Failed");
    for addr in addrs {
        if let SocketAddr::V4(_) = addr {
            return addr;
        }
    }
    unreachable!("Cannot lookup address");
}


fn main() {
    let mut event_loop = core::reactor::Core::new().unwrap();
    let handle = event_loop.handle();

    let opts = MqttOptions::new();
    //let addr = lookup_ipv4("localhost:1883");

    let mut connection = Connection::start(opts, None, None).unwrap();
    connection.run();
    thread::sleep(Duration::new(20, 0));
}
