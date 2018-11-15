use futures::{sink::Sink, stream::Stream, Poll, StartSend};

use client::prepend::prepend::Prepend;
use error::{NetworkError, PollError};
use futures::Async;
use mqtt3::Packet;
use std::io;

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

//TODO: Remove Option and use Chain stream directly
#[must_use = "streams do nothing unless polled"]
pub struct MqttStream<S1, S2, S3>
    where S3: Stream
{
    network_stream: S1,
    network_sink: S2,
    request_stream: Option<Prepend<S3>>,
    flag: bool,
}

pub fn new<S1, S2, S3>(network_stream: S1, network_sink: S2, request_stream: Prepend<S3>) -> MqttStream<S1, S2, S3>
    where S1: Stream<Item = Packet, Error = NetworkError>,
          S2: Sink<SinkItem = Packet, SinkError = io::Error>,
          S3: Stream<Item = Packet, Error = NetworkError>
{
    MqttStream { network_stream,
                 network_sink,
                 request_stream: Some(request_stream),
                 flag: true }
}

impl<S1, S2, S3> MqttStream<S1, S2, S3>
    where S1: Stream<Item = Packet, Error = NetworkError>,
          S2: Sink<SinkItem = Packet, SinkError = io::Error>,
          S3: Stream<Item = Packet, Error = NetworkError>
{
    fn interleave(&mut self) -> Poll<Option<S1::Item>, NetworkError> {
        let user_request_stream = self.request_stream.as_mut().unwrap();
        let network_stream = &mut self.network_stream;

        let (a, b) = if self.flag {
            (user_request_stream as &mut Stream<Item = _, Error = _>, network_stream as &mut Stream<Item = _, Error = _>)
        } else {
            (network_stream as &mut Stream<Item = _, Error = _>, user_request_stream as &mut Stream<Item = _, Error = _>)
        };

        self.flag = !self.flag;

        let a_done = match a.poll()? {
            Async::Ready(Some(item)) => return Ok(Some(item).into()),
            Async::Ready(None) => true,
            Async::NotReady => false,
        };

        match b.poll()? {
            Async::Ready(Some(item)) => {
                // If the other stream isn't finished yet, give them a chance to
                // go first next time as we pulled something off `b`.
                if !a_done {
                    self.flag = !self.flag;
                }
                Ok(Some(item).into())
            }
            Async::Ready(None) if a_done => Ok(None.into()),
            Async::Ready(None) | Async::NotReady => Ok(Async::NotReady),
        }
    }
}

impl<S1, S2, S3> Stream for MqttStream<S1, S2, S3>
    where S1: Stream<Item = Packet, Error = NetworkError>,
          S2: Sink<SinkItem = Packet, SinkError = io::Error>,
          S3: Stream<Item = Packet, Error = NetworkError>
{
    type Item = Packet;
    type Error = PollError<S3>;

    fn poll(&mut self) -> Poll<Option<S1::Item>, PollError<S3>> {
        match self.interleave() {
            Ok(v) => Ok(v),
            Err(e) => {
                let stream = self.request_stream.take().unwrap();
                Err(PollError::Network((e, stream)))
            }
        }
    }
}

impl<S1, S2, S3> Sink for MqttStream<S1, S2, S3>
    where S1: Stream<Item = Packet, Error = NetworkError>,
          S2: Sink<SinkItem = Packet, SinkError = io::Error>,
          S3: Stream<Item = Packet, Error = NetworkError>
{
    type SinkItem = Packet;
    type SinkError = PollError<S3>;

    fn start_send(&mut self, item: S2::SinkItem) -> StartSend<S2::SinkItem, PollError<S3>> {
        self.network_sink.start_send(item).map_err(|e| {
                                              let stream = self.request_stream.take().unwrap();
                                              PollError::Network((NetworkError::Io(e), stream))
                                          })
    }

    fn poll_complete(&mut self) -> Poll<(), PollError<S3>> {
        self.network_sink.poll_complete().map_err(|e| {
                                             let stream = self.request_stream.take().unwrap();
                                             PollError::Network((NetworkError::Io(e), stream))
                                         })
    }
}
