use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct MqttOptions {
    /// broker address that you want to connect to
    broker_addr: String,
    /// keep alive time to send pingreq to broker when the connection is idle
    keep_alive: Option<u16>,
    /// clean (or) persistent session 
    clean_session: bool,
    /// sleep time before retrying for connection
    reconnect_after: Option<u16>,
    /// try reconnection even if the first connect fails
    first_reconnection_loop: bool,
    /// client identifier
    client_id: String,
    /// username and password
    credentials: Option<(String, String)>,
    /// ca cert and client cert, key (for client auth)
    certs: Option<(PathBuf, Option<(PathBuf, PathBuf)>)>,
    /// maximum packet size
    max_packet_size: usize,
}

impl MqttOptions {
    
    pub fn new(id: &str, addr: &str) -> MqttOptions {
        // TODO: Validate client id. Shouldn't be empty or start with spaces
        // TODO: Validate if addr is proper address type
        MqttOptions {
            broker_addr: addr.to_string(),
            keep_alive: Some(10),
            clean_session: false,
            client_id: id.to_string(),
            credentials: None,
            reconnect_after: None,
            first_reconnection_loop: false,
            certs: None,
            max_packet_size: 100 * 1024,
        }
    }

    /// Set number of seconds after which client should ping the broker
    /// if there is no other data exchange
    pub fn set_keep_alive(mut self, secs: u16) -> Self {
        if secs < 5 {
            panic!("Keep alives should be greater than 5 secs");
        }

        self.keep_alive = Some(secs);
        self
    }

    /// Set packet size limit (in Kilo Bytes)
    pub fn set_max_packet_size(mut self, sz: usize) -> Self {
        self.max_packet_size = sz * 1024;
        self
    }

    /// `clean_session = true` removes all the state from queues & instructs the broker 
    /// to clean all the client state when client disconnects. 
    ///
    /// When set `false`, broker will hold the client state and performs pending
    /// operations on the client when reconnection with same `client_id`
    /// happens. Local queue state is also held to retransmit packets after reconnection.
    ///
    /// So **make sure that you manually set `client_id` when `clean_session` is false**
    pub fn set_clean_session(mut self, clean_session: bool) -> Self {
        self.clean_session = clean_session;
        self
    }


    /// Set `username` for broker to perform client authentication
    /// via `username` and `password`
    pub fn set_credentials(mut self, username: &str, password: &str) -> Self {
        self.credentials = Some((username.to_string(), password.to_string()));
        self
    }

    /// Time interval after which client should retry for new
    /// connection if there are any disconnections. By default, no retry will happen
    pub fn set_reconnect_after(mut self, dur: u16) -> Self {
        self.reconnect_after = Some(dur);
        self
    }

    pub fn set_first_reconnection_loop(mut self, reconnect: bool) -> Self {
        self.first_reconnection_loop = reconnect;
        self
    }

    /// Set ca, client cert and key for server to do client authentication
    pub fn set_certs<P>(mut self, ca: P, client_certs: Option<(P, P)>) -> Self
    where P: AsRef<Path>,
    {
        let client_certs = match client_certs {
            Some((cert, key)) => Some((cert.as_ref().to_path_buf(), key.as_ref().to_path_buf())),
            None => None,
        };
        
        self.certs = Some((ca.as_ref().to_path_buf(), client_certs));
        self
    }
}