use failure;
//use futures::sync::mpsc::Receiver;
use futures::{Future, Sink, Stream};
use std::net::SocketAddr;
use std::thread;
use tokio::net::TcpStream;
use tokio_timer::Deadline;
use codec::MqttCodec;
use error::ConnectError;
use futures::future;
use futures::future::FutureResult;
use futures::lazy;
use mqtt3::ConnectReturnCode;
use mqtt3::Packet;
use mqttoptions::MqttOptions;
use tokio_codec::{Decoder, Framed};
use tokio_io::AsyncRead;
use tokio_core::reactor::Core;

use tokio::runtime::current_thread;
use tokio_current_thread;
use pretty_env_logger;
use std::time::Instant;
use std::time::Duration;

pub struct Connection;

fn tcp_connect(address: &SocketAddr) -> impl Future<Item = Framed<TcpStream, MqttCodec>, Error = ConnectError> {
    TcpStream::connect(address)
        .map_err(ConnectError::from)
        .map(|stream| MqttCodec.framed(stream))
}

fn handshake(framed: Framed<TcpStream, MqttCodec>) -> impl Future<Item = Framed<TcpStream, MqttCodec>, Error = ConnectError> {
    let mqttoptions = MqttOptions::default();
    let connect_packet = mqttoptions.connect_packet();

    let framed = framed.send(connect_packet);
    framed.map_err(|err| ConnectError::from(err))
}

fn mqtt_connect(address: &SocketAddr) {
    let mqtt_connect = tcp_connect(address).and_then(|framed| {
        handshake(framed).and_then(|framed| {
            framed
                .into_future()
                .map_err(|(err, _framed)| ConnectError::from(err))
                .and_then(|(response, framed)| validate_connack(response.unwrap(), framed))
        })
    });

    // let mqtt_connect_deadline = Deadline::new(mqtt_connect, Instant::now() + Duration::from_secs(10));

    let _framed = match tokio_current_thread::block_on_all(mqtt_connect) {
        Ok(v) => v,
        Err(e) => panic!("{:?}", e),
    };

    thread::sleep_ms(15000);
}

fn validate_connack(packet: Packet, framed: Framed<TcpStream, MqttCodec>) -> FutureResult<Framed<TcpStream, MqttCodec>, ConnectError> {
    info!("Packet = {:?}", packet);

    match packet {
        Packet::Connack(connack) => {
            if connack.code == ConnectReturnCode::Accepted {
                future::ok(framed)
            } else {
                future::err(ConnectError::MqttConnectionRefused(connack.code.to_u8()))
            }
        }
        _ => unimplemented!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use tokio::runtime::current_thread;

    #[test]
    fn it_works() {
        pretty_env_logger::init();
        mqtt_connect(&"127.0.0.1:1883".parse().unwrap());
        thread::sleep_ms(10000);
    }
}
