use std::result;
use std::io;
use std::sync::mpsc::{TryRecvError, TrySendError, SendError};
use std::net::TcpStream;

use openssl;
use mqtt::topic_name::TopicNameError;
use mqtt::topic_filter::TopicFilterError;
use mqtt::packet::*;
use mqtt::control::variable_header::ConnectReturnCode;
use connection::NetworkRequest;

pub type SslError = openssl::error::ErrorStack;
pub type HandShakeError = openssl::ssl::HandshakeError<TcpStream>;
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
        Ssl(e: SslError) {
            from()
        }
        Handshake(e: HandShakeError) {
            from()
        }
        ConnectionRefused(e: ConnectReturnCode)
    }
}

impl<'a, P: Packet<'a>> From<PacketError<'a, P>> for Error {
    fn from(_: PacketError<'a, P>) -> Error { Error::MqttPacket }
}
