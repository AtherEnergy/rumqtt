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
///
///

#[must_use = "streams do nothing unless polled"]
pub struct MqttStream<S1, S2, S3> {
    network_stream: S1,
    network_sink: S2,
    user_request_stream: Option<S3>,
    network_stream_done: bool,
    user_request_stream_done: bool,
    flag: bool,
}

pub fn new<S1, S2, S3>(network_stream: S1, network_sink: S2, user_request_stream: S3) -> MqttStream<S1, S2, S3>
    where S1: Stream<Item = Packet, Error = NetworkError>,
          S2: Sink<SinkItem = Packet, SinkError = io::Error>,
          S3: Stream<Item = Packet, Error = NetworkError>
{
    MqttStream { network_stream,
                 network_sink,
                 user_request_stream: Some(user_request_stream),
                 network_stream_done: false,
                 user_request_stream_done: false,
                 flag: true }
}

//macro_rules! request_stream_ready {
//    ($e:expr) => (match $e {
//        Ok(Async::Ready(Some(item))) => return Ok(Some(item).into()),
//        Ok(Async::Ready(None)) => true,
//        Ok(Async::NotReady) => return Ok(Async::NotReady),
//        Err(e) => return Err(From::from(e)),
//    })
//}

macro_rules! request_stream_poll {
    //let user_request = $e.unwrap();
    ($e:expr) => {{
        let mut user_request = $e.as_mut().unwrap();
        match user_request.poll() {
            Ok(Async::Ready(Some(item))) => return Ok(Some(item).into()),
            Ok(Async::Ready(None)) => true,
            Ok(Async::NotReady) => return Ok(Async::NotReady),
            Err(e) => return Err(From::from(e)),
        }
    }}
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
            self.flag = !self.flag;

            let done = match self.network_stream.poll() {
                Ok(Async::Ready(Some(item))) => return Ok(Some(item).into()),
                Ok(Async::Ready(None)) => {
                    let stream = self.user_request_stream.take().unwrap();
                    return Err(PollError::StreamClosed(stream))
                },
                Ok(Async::NotReady) => false,
                Err(e) => return Err(From::from(e))
            };

            // end the user request stream if it's done and don't poll it again
            match self.user_request_stream.as_mut().unwrap().poll() {
                // poll network first next time again if user stream returns a value
                Ok(Async::Ready(Some(item))) => {
                    self.flag = !self.flag;
                    Ok(Some(item).into())
                },
                // both done. no need to poll again
                Ok(Async::Ready(None)) if done => Ok(None.into()),
                // done or not ready but above stream not done. poll again
                Ok(Async::Ready(None)) | Ok(Async::NotReady) => Ok(Async::NotReady),
                Err(e) => {
                    let stream = self.user_request_stream.take().unwrap();
                    Err(PollError::UserRequest(e))
                }
            }
        } else {
            self.flag = !self.flag;

            let done = match self.user_request_stream.as_mut().unwrap().poll() {
                Ok(Async::Ready(Some(item))) => return Ok(Some(item).into()),
                Ok(Async::Ready(None)) => true,
                Ok(Async::NotReady) => false,
                Err(e) => return Err(From::from(e))
            };

            match self.network_stream.poll() {
                // poll user request first next time again if n/w returns a value
                Ok(Async::Ready(Some(item))) => {
                    if !done {
                        self.flag = !self.flag;
                    }
                    Ok(Some(item).into())
                },
                Ok(Async::Ready(None)) => {
                    let stream = self.user_request_stream.take().unwrap();
                    return Err(PollError::StreamClosed(stream))
                },
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Err(e) => return Err(From::from(e))
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
            let stream = self.user_request_stream.take().unwrap();
            PollError::Network((NetworkError::Io(e), stream))
        })
    }

    fn poll_complete(&mut self) -> Poll<(), PollError<S3>> {
        self.network_sink.poll_complete().map_err(|e| {
            let stream = self.user_request_stream.take().unwrap();
            PollError::Network((NetworkError::Io(e), stream))
        })
    }
}
