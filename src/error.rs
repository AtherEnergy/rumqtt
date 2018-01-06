use std::result;
use std::io;
use std::sync::mpsc::{TryRecvError, TrySendError, SendError};

use mqtt::topic_name::TopicNameError;
use mqtt::topic_filter::TopicFilterError;
use mqtt::packet::*;
use mqtt::control::variable_header::ConnectReturnCode;
use connection::NetworkRequest;

use stream::StreamError;

pub type Result<T> = result::Result<T, Error>;

quick_error! {
    #[derive(Debug)]
    pub enum Error {
        Io(err: io::Error) {
            from()
            description("io error")
            display("I/O error: {}", err)
            cause(err)
        }
        TrySend(err: TrySendError<NetworkRequest>) {
            from()
        }
        TryRecv(err: TryRecvError) {
            from()
        }
        Send(err: SendError<NetworkRequest>) {
            from()
        }
        TopicName(err: TopicNameError) {
            from()
        }
        TopicFilter(err: TopicFilterError) {
            from()
        }
        ConnectionAbort
        HandshakeFailed
        InvalidState
        InvalidPacket
        Packet
        MqttPacket
        PingTimeout
        AwaitPingResp
        StreamError(e: StreamError) {
            from()
        }
        ConnectionRefused(e: ConnectReturnCode)
    }
}

impl<'a, P: Packet<'a>> From<PacketError<'a, P>> for Error {
    fn from(_: PacketError<'a, P>) -> Error { Error::MqttPacket }
}
