use std::result;
use std::io;
use mqtt::topic_name::TopicNameError;
use mio;

pub type Result<T> = result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    InvalidMosqClient,
    ConnectionError(i32),
    MqttEncodeError,
    TlsError(i32),
    SubscribeError(i32),
    PublishError(i32),
    AlreadyConnected,
    UnsupportedFeature,
    UnrecognizedPacket,
    ConnectionAbort,
    HandshakeFailed,
    ProtocolViolation,
    Disconnected,
    Timeout,
    ConnectionRefused(i32),
    Io(io::Error),
    InvalidCert(String),
    NoStreamError,
    TopicNameError,
    NoReconnectTry,
    MqttPacketError,
    MioNotifyError,
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::Io(err)
    }
}


impl From<TopicNameError> for Error {
    fn from(_: TopicNameError) -> Error {
        Error::TopicNameError
    }
}

impl<T> From<mio::NotifyError<T>> for Error {
    fn from(_: mio::NotifyError<T>) -> Error {
        Error::MioNotifyError
    }
}
