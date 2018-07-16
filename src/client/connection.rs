use failure;
//use futures::sync::mpsc::Receiver;
use futures::{Future, Sink, Stream};
use std::net::SocketAddr;
use std::thread;
use tokio::net::TcpStream;
use tokio::timer::Deadline;
use codec::MqttCodec;
use error::ConnectError;
use futures::future;
use futures::future::FutureResult;
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
use tokio;


/// Composes a future which makes a new tcp connection to the broker.
/// Note this doesn't actual connect to the broker
fn tcp_connect_future(address: &SocketAddr) -> impl Future<Item = Framed<TcpStream, MqttCodec>, Error = ConnectError> {
    TcpStream::connect(address)
        .map_err(ConnectError::from)
        .map(|stream| MqttCodec.framed(stream))
}

/// Composes a future which sends mqtt connect packet to the broker.
/// Note that this doesn't actually send the connect packet.
fn handshake_future(framed: Framed<TcpStream, MqttCodec>) -> impl Future<Item = Framed<TcpStream, MqttCodec>, Error = ConnectError> {
    let mqttoptions = MqttOptions::default();
    let connect_packet = mqttoptions.connect_packet();

    let framed = framed.send(Packet::Connect(connect_packet));
    framed.map_err(|err| ConnectError::from(err))
}

/// Composes a new future which is a combination of tcp connect + handshake + connack receive.
/// This function also runs to eventloop to create mqtt connection and returns `Framed`
fn mqtt_connect(address: &SocketAddr) -> Result<Framed<TcpStream, MqttCodec>, ConnectError> {
    let mqtt_connect_deadline = tcp_connect_future(address).and_then(|framed| {
        handshake_future(framed).and_then(|framed| {
            framed
                .into_future()
                .map_err(|(err, _framed)| ConnectError::from(err))
                .and_then(|(response, framed)| validate_connack(response.unwrap(), framed))
        })
    });

    // TODO: Add a timeout to the whole tcp connect + mqtt connect + connack wait so that our client
    // TODO: won't be indefinitely blocked
    // let mqtt_connect_deadline = Deadline::new(mqtt_connect_deadline, Instant::now() + Duration::from_secs(30));

    tokio_current_thread::block_on_all(mqtt_connect_deadline)
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

//  NOTES: Don't use `wait` in eventloop thread even if you
//         are ok with blocking code. It might cause deadlocks
// https://github.com/tokio-rs/tokio-core/issues/182


struct Connection {

}

//pub fn run(framed: Framed<TcpStream, MqttCodec>) {
//    let (network_sink, network_stream) = framed.split();
//
//    let mut connection = Connection;
//    let network_reader = network_stream
//    .for_each(move |packet| {
//        connection;
//        future::ok(())
//    })
//    .map_err(|e| future::err::<(), _>(connection));
//}


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
