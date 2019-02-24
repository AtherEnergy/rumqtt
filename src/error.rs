//! All errors
use crate::client::{Command, Request};
use crossbeam_channel::RecvError;
use derive_more::From;
use failure::Fail;
use futures::sync::mpsc::SendError;
#[cfg(feature = "jwt")]
use jsonwebtoken;
use mqtt311::Packet;
use std::io::Error as IoError;
use tokio::timer::{self, timeout};

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Fail, From)]
pub enum ClientError {
    #[fail(display = "No subscriptions")]
    ZeroSubscriptions,
    #[fail(display = "Packet size limit has crossed maximum")]
    PacketSizeLimitExceeded,
    #[fail(display = "Client id should not be empty")]
    EmptyClientId,
    #[fail(display = "Failed sending request to connection thread. Error = {}", _0)]
    MpscRequestSend(SendError<Request>),
    #[fail(display = "Failed sending request to connection thread. Error = {}", _0)]
    MpscCommandSend(SendError<Command>),
}

#[derive(Debug, Fail, From)]
pub enum MqttError {
    #[fail(display = "Connection failed")]
    ConnectError,
    #[fail(display = "Network call failed")]
    NetworkError,
}

// TODO: Modify mqtt311 to return enums for mqtt connect error
#[derive(Debug, Fail, From)]
pub enum ConnectError {
    #[fail(display = "Mqtt connection failed. Error = {}", _0)]
    MqttConnectionRefused(u8),
    #[cfg(feature = "jwt")]
    #[fail(display = "Mqtt connection failed. Error = {}", _0)]
    Jwt(jsonwebtoken::errors::Error),
    #[fail(display = "Io failed. Error = {}", _0)]
    Io(IoError),
    #[fail(display = "Receiving connection status failed. Error = {}", _0)]
    Recv(RecvError),
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

#[derive(Debug, Fail, From)]
pub enum NetworkError {
    #[fail(display = "Io failed. Error = {}", _0)]
    Io(IoError),
    #[fail(display = "Last ping response not received")]
    AwaitPingResp,
    #[fail(display = "Client not in connected state")]
    InvalidState,
    #[fail(display = "Couldn't ping in time")]
    Timeout,
    #[fail(display = "Received unsolicited acknowledgment")]
    Unsolicited,
    #[fail(display = "Tokio timer error = {}", _0)]
    Timer(timer::Error),
    #[fail(display = "Tokio timer error = {}", _0)]
    TimeOut(timeout::Error<IoError>),
    #[fail(display = "Tokio timer error")]
    ThrottleError,
    #[fail(display = "User requested for reconnect")]
    UserReconnect,
    #[fail(display = "User requested for disconnect")]
    UserDisconnect,
    #[fail(display = "Network stream closed")]
    NetworkStreamClosed,
    #[fail(display = "Throttle error while rate limiting")]
    Throttle,
    #[fail(display = "Dummy error for converting () to network error")]
    Blah,
}
