use crate::client::{Command, Request};
use crossbeam_channel::RecvError;
use derive_more::From;
use failure::Fail;
use futures::sync::mpsc::SendError;
#[cfg(feature = "jwt")]
use jsonwebtoken;
use mqtt311::Packet;
use std::io::Error as IoError;
use tokio_timer::{self, timeout};

/// Possible client errors while sending requests and commmands
/// to the eventloop
#[derive(Debug, Fail, From)]
pub enum ClientError {
    /// List of subscriptions in the `subscribe` method should be > 0
    #[fail(display = "No subscriptions")]
    ZeroSubscriptions,
    /// Publish payload size is greater than the limit set
    #[fail(display = "Packet size limit has crossed maximum")]
    PacketSizeLimitExceeded,
    /// Client id empty
    #[fail(display = "Client id should not be empty")]
    EmptyClientId,
    /// Failed to send request to the event loop
    #[fail(display = "Failed sending request to connection thread. Error = {}", _0)]
    MpscRequestSend(SendError<Request>),
    /// Failed to send command to the event loop
    #[fail(display = "Failed sending request to connection thread. Error = {}", _0)]
    MpscCommandSend(SendError<Command>),
}

// TODO: Modify mqtt311 to return enums for mqtt connect error
/// Possible errors while attempting a connection. Returned by the[start]
/// method during initial connection depending on the [reconnect option] set
/// 
/// [start]: client/struct.MqttClient.html#method.start
/// [reconnect option]: mqttoptions/enum.ReconnectOptions.html
#[derive(Debug, Fail, From)]
pub enum ConnectError {
    /// Mqtt connection refused by the broker
    #[fail(display = "Mqtt connection failed. Error = {}", _0)]
    MqttConnectionRefused(u8),
    /// Json web token error
    #[cfg(feature = "jwt")]
    #[fail(display = "Jwt error. Error = {}", _0)]
    Jwt(jsonwebtoken::errors::Error),
    /// IO Error
    #[fail(display = "Io failed. Error = {}", _0)]
    Io(IoError),
    /// Error in receiving connection status
    #[fail(display = "Receiving connection status failed. Error = {}", _0)]
    Recv(RecvError),
    /// Mqtt connection timeout
    #[fail(display = "Couldn't create mqtt connection in time")]
    Timeout,
    /// Unexpected incoming packet while waiting for connack
    #[fail(display = "Unsolicited packet while waiting for connack. Recived = {:?}", _0)]
    NotConnackPacket(Packet),
    /// No response
    #[fail(display = "Empty response")]
    NoResponse,
    /// No ca set while creating a TLS connection
    #[fail(display = "Builder doesn't contain certificate authority")]
    NoCertificateAuthority,
}

/// Networks errors while handling mqtt io
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
    Timer(tokio_timer::Error),
    #[fail(display = "Tokio timer error = {}", _0)]
    TimeOut(timeout::Error<IoError>),
    #[fail(display = "User requested for reconnect")]
    UserReconnect,
    #[fail(display = "User requested for disconnect")]
    UserDisconnect,
    #[fail(display = "Network stream closed")]
    NetworkStreamClosed,
    #[fail(display = "Dummy error for converting () to network error")]
    Blah,
}
