use std::io;
use mqtt3;

quick_error! {
    #[derive(Debug, PartialEq)]
    pub enum PingError {
        // when last ping response isn't received
        AwaitPingResp
        // client not in connected state
        InvalidState
        // did not ping with in time
        Timeout
    }
}

quick_error! {
    #[derive(Debug, PartialEq)]
    pub enum ConnectError {
        MqttConnectionRefused(e: mqtt3::ConnectReturnCode) {
            from()
        }
    }
}

quick_error! {
    #[derive(Debug, PartialEq)]
    pub enum PublishError {
        PacketSizeLimitExceeded
        InvalidState
    }
}

quick_error! {
    #[derive(Debug, PartialEq)]
    pub enum PubackError {
        Unsolicited
    }
}