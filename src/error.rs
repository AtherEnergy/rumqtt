use std::result;
use std::io;
use std::sync::mpsc;
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
    IncommingStorageAbsent,
    OutgoingStorageAbsent,
    HandshakeFailed,
    ProtocolViolation,
    Disconnected,
    Timeout,
    ConnectionRefused(i32),
    Io(io::Error),
    InvalidCert(String),
    NoStreamError,
    TopicNameError,
    Mpsc(mpsc::RecvError),
    NoReconnectTry,
    MqttPacketError,
    MioError(mio::NotifyError),
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::Io(err)
    }
}

impl From<mpsc::RecvError> for Error {
    fn from(err: mpsc::RecvError) -> Error {
        Error::Mpsc(err)
    }
}

impl From<TopicNameError> for Error {
    fn from(_: TopicNameError) -> Error {
        Error::TopicNameError
    }
}

impl From<mio::NotifyError> for Error {
    fn from(e: mio::NotifyError) -> Error {
        Error::MioError(e)
    }
}
