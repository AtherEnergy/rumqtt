use futures::StartSend;
use futures::Poll;
use futures::stream::Stream;
use futures::sink::Sink;

use error::NetworkError;
use error::PollError;
use std::io;
use futures::Async;
use mqtt3::Packet;

/// Customized stream/sink to cater rumqtt needs. 
/// 1
/// ------
/// This implementation returns channel back to the user when there are errors.
/// This simplifies ownership and handling of pending requests in the queue as we are going to
/// reuse same channel.
/// 2
/// ------
/// `Select` on 2 streams will continue the 2nd stream even after the first stream ends. In our
/// case we need to detect disconnections as soon as server closes the connection. This alters
/// the select implementation to throw error when `network_stream` closes. 
/// (by default close = stream end)
/// 3
/// ------
/// Special user command like `pause` should immediately disable network activity.
/// Rate limiting might be a good future feature

#[must_use = "streams do nothing unless polled"]
pub struct MqttStream<S1, S2, S3> {
    network_stream: S1,
    network_sink: S2,
    userrequest_rx: Option<S3>,
    flag: bool,
}

pub fn new<S1, S2, S3>(network_stream: S1, network_sink: S2, userrequest_rx: S3) -> MqttStream<S1, S2, S3>
    where S1: Stream<Item = Packet, Error = NetworkError>,
          S2: Sink<SinkItem = Packet, SinkError = io::Error>, 
          S3: Stream<Item = Packet, Error = NetworkError>
{
    MqttStream { network_stream,
                 network_sink,
                 userrequest_rx: Some(userrequest_rx),
                 flag: false }
}

impl<S1, S2, S3> Stream for MqttStream<S1, S2, S3>
    where S1: Stream<Item = Packet, Error = NetworkError>,
          S2: Sink<SinkItem = Packet, SinkError = io::Error>, 
          S3: Stream<Item = Packet, Error = NetworkError>
{
    type Item = Packet;
    type Error = PollError<S3>;

    fn poll(&mut self) -> Result<Async<Option<Packet>>, PollError<S3>> {
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
                    Err(PollError::Stream((e, stream)))
                }
            }
        } else {
            let a_done = match self.network_stream.poll() {
                Ok(Async::Ready(Some(item))) => return Ok(Some(item).into()),
                Ok(Async::Ready(None)) => true,
                Ok(Async::NotReady) => false,
                Err(e) => {
                    let stream = self.userrequest_rx.take().unwrap();
                    return Err(PollError::Stream((e, stream)));
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

impl<S1, S2, S3> Sink for MqttStream<S1, S2, S3>
    where S1: Stream<Item = Packet, Error = NetworkError>,
          S2: Sink<SinkItem = Packet, SinkError = io::Error>, 
          S3: Stream<Item = Packet, Error = NetworkError> {

    type SinkItem = Packet;
    type SinkError = PollError<S3>;

    fn start_send(&mut self, item: S2::SinkItem) -> StartSend<S2::SinkItem, PollError<S3>> {
        self.network_sink.start_send(item).map_err(|e| {
            let stream = self.userrequest_rx.take().unwrap();
            PollError::Stream((NetworkError::Io(e), stream))
        })
    }

    fn poll_complete(&mut self) -> Poll<(), PollError<S3>> {
        self.network_sink.poll_complete().map_err(|e| {
            let stream = self.userrequest_rx.take().unwrap();
            PollError::Stream((NetworkError::Io(e), stream))
        })
    }
} 
