use std::result;
use std::io;
use mqtt::topic_name::TopicNameError;
use mqtt::topic_filter::TopicFilterError;
use mqtt::packet::*;
use mqtt::control::variable_header::ConnectReturnCode;
use std::sync::mpsc::{RecvError, SendError};
use openssl::ssl;
use mio;

pub type Result<T> = result::Result<T, Error>;
pub type SslError = ssl::error::SslError;

#[derive(Debug)]
pub enum Error {
    InvalidMosqClient,
    MqttEncode,
    Tls(i32),
    Subscribe(i32),
    Publish(i32),
    AlreadyConnected,
    UnrecognizedPacket,
    ConnectionAbort,
    HandshakeFailed,
    Disconnected,
    Timeout,
    ConnectionRefused(ConnectReturnCode),
    Io(io::Error),
    InvalidCert(String),
    NoStream,
    TopicName,
    TopicFilter,
    NoReconnectTry,
    MqttPacket,
    MioNotify,
    Ssl,
    EventLoop,
    MioNotifyError,
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
    fn from(_: SendError<T>) -> Error { Error::MioNotify }
}

impl From<RecvError> for Error {
    fn from(_: RecvError) -> Error { Error::MioNotify }
}

impl<'a, T: Packet<'a>> From<PacketError<'a, T>> for Error {
    fn from(_: PacketError<'a, T>) -> Error { Error::MqttPacket }
}

impl From<SslError> for Error {
    fn from(_: SslError) -> Error { Error::Ssl }
}

impl<T> From<mio::NotifyError<T>> for Error {
    fn from(_: mio::NotifyError<T>) -> Error { Error::MioNotifyError }
}
