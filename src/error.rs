use std::result;
use std::io;

pub type Result<T> = result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
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
    // UnhandledPuback(PacketIdentifier),
    // UnhandledPubrec(PacketIdentifier),
    // UnhandledPubrel(PacketIdentifier),
    // UnhandledPubcomp(PacketIdentifier),
    // ConnectionRefused(ConnectReturnCode),
    // Storage(StorageError),
    // Mqtt(MqttError),
    Io(io::Error),
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::Io(err)
    }
}
