use std::io::Error as IoError;

#[derive(Debug, Fail)]
pub enum ClientError {
    #[fail(display = "No subscriptions")]
    ZeroSubscriptions,
    #[fail(display = "Packet size limit has crossed maximum")]
    PacketSizeLimitExceeded,
    #[fail(display = "Client id should not be empty")]
    EmptyClientId
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

impl From<IoError> for ConnectError {
    fn from(err: IoError) -> ConnectError {
        ConnectError::Io(err)
    }
}