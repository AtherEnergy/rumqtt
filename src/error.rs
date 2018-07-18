use std::io::Error as IoError;

#[derive(Debug, Fail)]
pub enum ClientError {
    #[fail(display = "No subscriptions")]
    ZeroSubscriptions,
    #[fail(display = "Packet size limit has crossed maximum")]
    PacketSizeLimitExceeded,
    #[fail(display = "Client id should not be empty")]
    EmptyClientId,
}

#[derive(Debug, Fail)]
pub enum ConnectError {
    #[fail(display = "Mqtt connection failed. Error = {}", _0)]
    MqttConnectionRefused(u8),
    #[fail(display = "Io failed. Error = {}", _0)]
    Io(IoError),
    #[fail(display = "Empty dns list")]
    DnsListEmpty,
    #[fail(display = "Received halt command")]
    Halt,
    #[fail(display = "Outgoing receive failed")]
    Outgoing,
    #[fail(display = "Counldn't receive connack in time")]
    Timeout,
}

#[derive(Debug, Fail)]
pub enum NetworkReceiveError {
    #[fail(display = "Io failed. Error = {}", _0)]
    Io(IoError),
    #[fail(display = "Received unsolicited acknowledgment")]
    Unsolicited
}

#[derive(Debug, Fail)]
pub enum NetworkSendError {
    #[fail(display = "Io failed. Error = {}", _0)]
    Io(IoError),
    #[fail(display = "Last ping response not received")]
    AwaitPingResp,
    #[fail(display = "Client not in connected state")]
    InvalidState,
    #[fail(display = "Couldn't ping in time")]
    Timeout,
    #[fail(display = "Packet limit size exceeded")]
    PacketSizeLimitExceeded
}

impl From<IoError> for ConnectError {
    fn from(err: IoError) -> ConnectError {
        ConnectError::Io(err)
    }
}

impl From<IoError> for NetworkReceiveError {
    fn from(err: IoError) -> NetworkReceiveError {
        NetworkReceiveError::Io(err)
    }
}

impl From<IoError> for NetworkSendError {
    fn from(err: IoError) -> NetworkSendError {
        NetworkSendError::Io(err)
    }
}
