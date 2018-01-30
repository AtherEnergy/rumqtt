use futures::sync::mpsc::SendError;
use mqtt3::Packet;
use std::io::Error as IoError;
use openssl;
use client::Command;

#[cfg(feature = "jwt")]
use jwt;

#[derive(Debug, Fail)]
pub enum ClientError {
    #[fail(display = "No subscriptions")]
    ZeroSubscriptions,
    #[fail(display = "Packet size limit has crossed maximum")]
    PacketSizeLimitExceeded,
    #[fail(display = "Failed sending request to connection thread. Error = {}", _0)]
    MpscSend(SendError<Command>),
    #[fail(display = "Client id should not be empty")]
    EmptyClientId
}

// #[derive(Debug, Fail)]
// pub enum StateError {
//     #[fail(display = "Ping failed. Error = {}", error)]
//     Ping {error: PingError},
//     #[fail(display = "Publish failed. Error = {}", error)]
//     Publish {error: PublishError},
//     #[fail(display = "Subscribe failed. Error = {}", error)]
//     Subscribe {error: SubscribeError},
//     #[fail(display = "Puback failed. Error = {}", error)]
//     Puback {error: PubackError},
// }

#[derive(Debug, Fail, PartialEq)]
pub enum PingError {
    #[fail(display = "Last ping response not received")]
    AwaitPingResp,
    #[fail(display = "Client not in connected state")]
    InvalidState,
    #[fail(display = "Couldn't ping in time")]
    Timeout
}

#[derive(Debug, Fail)]
pub enum ConnectError {
    #[fail(display = "Mqtt connection failed. Error = {}", _0)]
    MqttConnectionRefused(u8),
    #[fail(display = "Io failed. Error = {}", _0)]
    Io(IoError),
    #[fail(display = "Tls failed. Error = {}", _0)]
    Tls(openssl::error::ErrorStack),
    #[cfg(feature = "jwt")]
    #[fail(display = "Jwt creation failed")]
    Jwt,
    #[fail(display = "Empty dns list")]
    DnsListEmpty,
    #[fail(display = "Received halt command")]
    Halt,
    #[fail(display = "Outgoing receive failed")]
    Outgoing,
}

#[derive(Debug, Fail, PartialEq)]
pub enum PublishError {
    #[fail(display = "Client not in connected state")]
    InvalidState,
    #[fail(display = "Packet limit size exceeded")]
    PacketSizeLimitExceeded
}

#[derive(Debug, Fail)]
pub enum PubackError {
    // #[fail(display = "Client not in connected state")]
    // InvalidState,
    #[fail(display = "Received unsolicited acknowledgment")]
    Unsolicited
}

#[derive(Debug, Fail)]
pub enum SubscribeError {
    #[fail(display = "Client not in connected state")]
    InvalidState
}

impl From<SendError<Command>> for ClientError {
    fn from(err: SendError<Command>) -> ClientError {
        ClientError::MpscSend(err)
    }
}

impl From<IoError> for ConnectError {
    fn from(err: IoError) -> ConnectError {
        ConnectError::Io(err)
    }
}

impl From<openssl::error::ErrorStack> for ConnectError {
    fn from(err: openssl::error::ErrorStack) -> ConnectError {
        ConnectError::Tls(err)
    }
}

#[cfg(feature = "jwt")]
impl From<jwt::errors::Error> for ConnectError {
    fn from(err: jwt::errors::Error) -> ConnectError {
        error!("Jwt failed. Error = {:?}", err);
        ConnectError::Jwt
    }
}