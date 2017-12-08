use futures::sync::mpsc::SendError;
use client::Request;
use std::io::Error as IoError;

#[derive(Debug, Fail)]
pub enum ClientError {
    #[fail(display = "No subscriptions")]
    ZeroSubscriptions,
    #[fail(display = "Packet size limit has crossed maximum")]
    PacketSizeLimitExceeded,
    #[fail(display = "Failed sending request to connection thread. Error = {}", _0)]
    MpscSend(SendError<Request>)
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

#[derive(Debug, Fail)]
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
    Io(IoError)
}

#[derive(Debug, Fail)]
pub enum PublishError {
    #[fail(display = "Client not in connected state")]
    InvalidState
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

impl From<SendError<Request>> for ClientError {
    fn from(err: SendError<Request>) -> ClientError {
        ClientError::MpscSend(err)
    }
}

impl From<IoError> for ConnectError {
    fn from(err: IoError) -> ConnectError {
        ConnectError::Io(err)
    }
}
