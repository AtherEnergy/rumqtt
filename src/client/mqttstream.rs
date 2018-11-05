use client::Request;
use codec::MqttCodec;
use futures::stream::Fuse;
use futures::stream::SplitStream;
use futures::stream::Stream;
use futures::sync::mpsc;
use tokio_codec::Framed;

use client::network::stream::NetworkStream;
use error::NetworkError;
use futures::future::Future;
use futures::Async;
use mqtt3::Packet;
use std::io;

#[must_use = "streams do nothing unless polled"]
pub struct MqttStream<S1, S2> {
    network_stream: S1,
    userrequest_rx: Option<S2>,
    flag: bool,
}

pub fn new<S1, S2>(network_stream: S1, userrequest_rx: S2) -> MqttStream<S1, S2>
    where S1: Stream<Item = Packet, Error = NetworkError>,
          S2: Stream<Item = Packet, Error = NetworkError>
{
    MqttStream { network_stream: network_stream,
                 userrequest_rx: Some(userrequest_rx),
                 flag: false }
}

impl<S1, S2> Stream for MqttStream<S1, S2>
    where S1: Stream<Item = Packet, Error = NetworkError>,
          S2: Stream<Item = Packet, Error = NetworkError>
{
    type Item = Packet;
    type Error = (NetworkError, S2);

    fn poll(&mut self) -> Result<Async<Option<Packet>>, (NetworkError, S2)> {
        if self.flag {
            let a_done = match self.userrequest_rx {
                Some(ref mut userrequest_rx) => match userrequest_rx.poll() {
                    Ok(Async::Ready(Some(item))) => return Ok(Some(item).into()),
                    Ok(Async::Ready(None)) => true,
                    Ok(Async::NotReady) => false,
                    Err(e) => panic!("Not possible"),
                },
                None => panic!("@@@"),
            };

            match self.network_stream.poll() {
                Ok(Async::Ready(Some(item))) => {
                    // If the other stream isn't finished yet, give them a chance to
                    // go first next time as we pulled something off `b`.
                    if !a_done {
                        self.flag = !self.flag;
                    }
                    Ok(Some(item).into())
                }
                Ok(Async::Ready(None)) if a_done => Ok(None.into()),
                Ok(Async::Ready(None)) | Ok(Async::NotReady) => Ok(Async::NotReady),
                Err(e) => {
                    let stream = self.userrequest_rx.take().unwrap();
                    Err((e, stream))
                }
            }
        } else {
            let a_done = match self.network_stream.poll() {
                Ok(Async::Ready(Some(item))) => return Ok(Some(item).into()),
                Ok(Async::Ready(None)) => true,
                Ok(Async::NotReady) => false,
                Err(e) => {
                    let stream = self.userrequest_rx.take().unwrap();
                    return Err((e, stream));
                }
            };

            match self.userrequest_rx {
                Some(ref mut userrequest_rx) => match userrequest_rx.poll() {
                    Ok(Async::Ready(Some(item))) => {
                        // If the other stream isn't finished yet, give them a chance to
                        // go first next time as we pulled something off `b`.
                        if !a_done {
                            self.flag = !self.flag;
                        }
                        Ok(Some(item).into())
                    }
                    Ok(Async::Ready(None)) if a_done => Ok(None.into()),
                    Ok(Async::Ready(None)) | Ok(Async::NotReady) => Ok(Async::NotReady),
                    Err(e) => panic!("Not possible"),
                },
                None => panic!("!!!"),
            }
        }
    }
}

//impl From<io::Error> for (NetworkError, Stream<Item = Packet, Error = NetworkError>) {
//    fn from(e: io::Error) -> Self {
//        (NetworkError::Io(e), Self)
//    }
//}
