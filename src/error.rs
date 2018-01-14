use futures::sync::mpsc::SendError;
use mqtt3::Packet;
use std::io::Error as IoError;
use native_tls;

#[derive(Debug, Fail)]
pub enum ClientError {
    #[fail(display = "No subscriptions")]
    ZeroSubscriptions,
    #[fail(display = "Packet size limit has crossed maximum")]
    PacketSizeLimitExceeded,
    #[fail(display = "Failed sending request to connection thread. Error = {}", _0)]
    MpscSend(SendError<Packet>),
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
    Tls(native_tls::Error),
    #[fail(display = "Empty dns list")]
    DnsListEmpty
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

impl From<SendError<Packet>> for ClientError {
    fn from(err: SendError<Packet>) -> ClientError {
        ClientError::MpscSend(err)
    }
}

impl From<IoError> for ConnectError {
    fn from(err: IoError) -> ConnectError {
        ConnectError::Io(err)
    }
}

impl From<native_tls::Error> for ConnectError {
    fn from(err: native_tls::Error) -> ConnectError {
        ConnectError::Tls(err)
    }
}
