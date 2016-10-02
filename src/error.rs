use std::result;
use std::io;
use std::sync::mpsc::{self, RecvError, TryRecvError};
use std::net::TcpStream;

use mio::timer::TimerError;
use mio::channel::SendError;
use openssl;
use mqtt::topic_name::TopicNameError;
use mqtt::topic_filter::TopicFilterError;
use mqtt::packet::*;
use mqtt::control::variable_header::ConnectReturnCode;


pub type SslError = openssl::error::ErrorStack;
pub type HandShakeError = openssl::ssl::HandshakeError<TcpStream>;
pub type Result<T> = result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    ConnectionAbort,
    ConnectionRefused(ConnectReturnCode),
    Io(io::Error),
    SendError,
    Recv(RecvError),
    TryRecv(TryRecvError),
    Timer(TimerError),
    TopicName,
    TopicFilter,
    NoReconnectTry,
    MqttPacket,
    Ssl(SslError),
    Timeout,
    InvalidPacket,
    InvalidState,
    HandshakeFailed,
    Handshake(HandShakeError),
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error { Error::Io(err) }
}

impl From<TopicNameError> for Error {
    fn from(_: TopicNameError) -> Error { Error::TopicName }
}

impl From<TopicFilterError> for Error {
    fn from(_: TopicFilterError) -> Error { Error::TopicFilter }
}

impl<T> From<SendError<T>> for Error {
    fn from(_: SendError<T>) -> Error { Error::SendError }
}

impl<T> From<mpsc::SendError<T>> for Error {
    fn from(_: mpsc::SendError<T>) -> Error { Error::SendError }
}

impl From<RecvError> for Error {
    fn from(err: RecvError) -> Error { Error::Recv(err) }
}

impl From<TryRecvError> for Error {
    fn from(err: TryRecvError) -> Error { Error::TryRecv(err) }
}

impl<'a, P: Packet<'a>> From<PacketError<'a, P>> for Error {
    fn from(_: PacketError<'a, P>) -> Error { Error::MqttPacket }
}

impl From<SslError> for Error {
    fn from(e: SslError) -> Error { Error::Ssl(e) }
}

impl From<HandShakeError> for Error {
    fn from(e: HandShakeError) -> Error { Error::Handshake(e) }
}

impl From<TimerError> for Error {
    fn from(e: TimerError) -> Error { Error::Timer(e) }
}
