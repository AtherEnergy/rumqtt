use mqtt3;
use futures::sync::mpsc::SendError;
use client::Request;

quick_error! {
    #[derive(Debug)]
    pub enum Error {
        MpscSend(e: SendError<Request>) {
            from()
        }
    }
}

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

quick_error! {
    #[derive(Debug, PartialEq)]
    pub enum SubscribeError {
    }
}

// quick_error! {
//     #[derive(Debug, PartialEq)]
//     pub enum SubackError {
//         // TODO: Add semi rejected error is some of the subscriptions are accepted
//         Rejected
//     }
// }