use client::Request;
use futures::sync::mpsc::SendError;
#[cfg(feature = "jwt")]
use jsonwebtoken;
use mqtt3::Packet;
use std::io::Error as IoError;
use tokio::timer;

#[derive(Debug, Fail)]
pub enum ClientError {
    #[fail(display = "No subscriptions")]
    ZeroSubscriptions,
    #[fail(display = "Packet size limit has crossed maximum")]
    PacketSizeLimitExceeded,
    #[fail(display = "Client id should not be empty")]
    EmptyClientId,
    #[fail(
        display = "Failed sending request to connection thread. Error = {}",
        _0
    )]
    MpscSend(SendError<Request>),
}

#[derive(Debug, Fail)]
pub enum MqttError {
    #[fail(display = "Connection failed")]
    ConnectError,
    #[fail(display = "Network call failed")]
    NetworkError,
}

// TODO: Modify mqtt311 to return enums for mqtt connect error
#[derive(Debug, Fail)]
pub enum ConnectError {
    #[fail(display = "Mqtt connection failed. Error = {}", _0)]
    MqttConnectionRefused(u8),
    #[cfg(feature = "jwt")]
    #[fail(display = "Mqtt connection failed. Error = {}", _0)]
    Jwt(jsonwebtoken::errors::Error),
    #[fail(display = "Io failed. Error = {}", _0)]
    Io(IoError),
    #[fail(display = "Empty dns list")]
    DnsListEmpty,
    #[fail(display = "Couldn't create mqtt connection in time")]
    Timeout,
    #[fail(
        display = "Unsolicited packet received while waiting for connack. Recived packet = {:?}",
        _0
    )]
    NotConnackPacket(Packet),
    #[fail(display = "Empty response")]
    NoResponse,
    #[fail(display = "Builder doesn't contain certificate authority")]
    NoCertificateAuthority,
}

#[derive(Debug, Fail)]
pub enum NetworkError {
    #[fail(display = "Io failed. Error = {}", _0)]
    Io(IoError),
    #[fail(display = "Last ping response not received")]
    AwaitPingResp,
    #[fail(display = "Client not in connected state")]
    InvalidState,
    #[fail(display = "Couldn't ping in time")]
    Timeout,
    #[fail(display = "Packet limit size exceeded")]
    PacketSizeLimitExceeded,
    #[fail(display = "Received unsolicited acknowledgment")]
    Unsolicited,
    #[fail(display = "Tokio timer error = {}", _0)]
    Timer(timer::Error),
    #[fail(display = "User requested for reconnect")]
    UserReconnect,
    #[fail(display = "User requested for disconnect")]
    UserDisconnect,
    #[fail(display = "Dummy error for converting () to network error")]
    Blah,
}

impl From<IoError> for ConnectError {
    fn from(err: IoError) -> ConnectError {
        ConnectError::Io(err)
    }
}

impl From<jsonwebtoken::errors::Error> for ConnectError {
    fn from(err: jsonwebtoken::errors::Error) -> ConnectError {
        ConnectError::Jwt(err)
    }
}

impl From<IoError> for NetworkError {
    fn from(err: IoError) -> NetworkError {
        NetworkError::Io(err)
    }
}

impl From<SendError<Request>> for ClientError {
    fn from(err: SendError<Request>) -> ClientError {
        ClientError::MpscSend(err)
    }
}

impl From<timer::Error> for NetworkError {
    fn from(err: timer::Error) -> NetworkError {
        NetworkError::Timer(err)
    }
}
