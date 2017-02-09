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

fn main() {
    let opts = MqttOptions::new();
    let mut connection = Connection::start(opts, None, None).unwrap();
    let e = connection.run();
    println!("{:?}", e);
}
