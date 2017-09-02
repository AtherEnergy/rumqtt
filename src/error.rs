use std::io;

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