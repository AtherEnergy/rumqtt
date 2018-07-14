use failure;
//use futures::sync::mpsc::Receiver;
use futures::{Future, Sink, Stream};
use std::net::SocketAddr;
use tokio::net::TcpStream;

use codec::MqttCodec;
use mqtt3::Packet;
use tokio_codec::{Framed, Decoder};
use tokio_io::AsyncRead;
use mqttoptions::MqttOptions;
use mqtt3::ConnectReturnCode;
use futures::future;
use error::ConnectError;
use futures::future::FutureResult;

pub struct Connection;

impl Connection {
    fn connect(address: &SocketAddr) -> impl Future<Item = Framed<TcpStream, MqttCodec>, Error = failure::Error> {
        TcpStream::connect(address)
            .map_err(failure::Error::from)
            .map(|stream| MqttCodec.framed(stream))
    }

    fn handshake(framed: Framed<TcpStream, MqttCodec>) -> impl Future<Item = impl Future<Item = Self, Error=ConnectError>,
                                                               Error = failure::Error> {
        let mqttoptions = MqttOptions::default();
        let connect_packet = mqttoptions.connect_packet();

        let framed = framed.send(connect_packet).map(|f| {
            f.into_future()
        });

        framed
            .map_err(|err|failure::Error::from(err))
            .map(|f|
                f.and_then(|(response, framed)| {
                    Self::validate_connack(response)
                })
            )
            .map_err(|(err, _framed)| {
                err
            })
    }

    fn validate_connack(response: Option<Packet>) -> FutureResult<Connection, ConnectError> {
        if let Some(packet) = response {
            match packet {
                Packet::Connack(connack) => {
                    if connack.code == ConnectReturnCode::Accepted {
                        future::ok(Connection)
                    } else {
                        future::err(ConnectError::MqttConnectionRefused(connack.code.to_u8()))
                    }
                }
                _ => unimplemented!()
            }
        } else {
            unimplemented ! ()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use tokio::runtime::current_thread;

    #[test]
    fn it_works() {
        let mut rt = current_thread::Runtime::new().unwrap();
        let mqtt = Connection::connect(&"127.0.0.1:1883".parse().unwrap());
        let mqtt = rt.block_on(mqtt).unwrap();
        thread::sleep_ms(10000);
    }
}
