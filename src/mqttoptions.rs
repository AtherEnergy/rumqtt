use mqtt3::{LastWill,
            Connect,
            Connack,
            Packet,
            Protocol};

use std::time::Duration;

/// Control how the connection is re-established if it is lost.
#[derive(Copy, Clone, Debug)]
pub enum ReconnectOptions {
    /// Don't automatically reconnect
    Never,
    /// Reconnect automatically if the connection has been established
    /// successfully at least once.
    ///
    /// Before a reconnection attempt, sleep for the specified amount of seconds.
    AfterFirstSuccess(u16),
    /// Always reconnect automatically.
    ///
    /// Before a reconnection attempt, sleep for the specified amount of seconds.
    Always(u16),
}

/// Configure server authentication.
#[derive(Clone, Debug)]
pub enum SecurityOptions {
    /// No authentication.
    None,
    /// Use the specified `(username, password)` tuple to authenticate.
    UsernamePassword((String, String))
}

#[derive(Clone, Debug)]
pub enum ConnectionMethod {
    Tcp,
    // ca and, optionally, a pair of client cert and client key
    Tls(String, Option<(String, String)>),
}

#[derive(Clone, Debug)]
pub struct MqttOptions {
    /// broker address that you want to connect to
    pub broker_addr: String,
    /// keep alive time to send pingreq to broker when the connection is idle
    pub keep_alive: Option<Duration>,
    /// clean (or) persistent session
    pub clean_session: bool,
    /// client identifier
    pub client_id: String,
    /// connection method
    pub connection_method: ConnectionMethod,
    /// reconnection options
    pub reconnect: ReconnectOptions,
    /// security options
    pub security: SecurityOptions,
    /// maximum packet size
    pub max_packet_size: usize,
    /// last will and testament
    pub last_will: Option<LastWill>,
}

impl Default for MqttOptions {
    fn default() -> Self {
        MqttOptions {
            broker_addr: "127.0.0.1:1883".into(),
            keep_alive: Some(Duration::from_secs(30)),
            clean_session: true,
            client_id: "test-client".into(),
            connection_method: ConnectionMethod::Tcp,
            reconnect: ReconnectOptions::AfterFirstSuccess(10),
            security: SecurityOptions::None,
            max_packet_size: 256 * 1024,
            last_will: None,
        }
    }
}

impl MqttOptions {
    pub fn new<S: Into<String>, T: Into<String>>(id: S, addr: T) -> MqttOptions {
        // TODO: Validate if addr is proper address type
        let id = id.into();
        if id.starts_with(" ") || id.is_empty() {
            panic!("Invalid client id")
        }

        MqttOptions {
            broker_addr: addr.into(),
            keep_alive: Some(Duration::from_secs(30)),
            clean_session: true,
            client_id: id.into(),
            connection_method: ConnectionMethod::Tcp,
            reconnect: ReconnectOptions::AfterFirstSuccess(10),
            security: SecurityOptions::None,
            max_packet_size: 256 * 1024,
            last_will: None,
        }
    }

    /// Set number of seconds after which client should ping the broker
    /// if there is no other data exchange
    pub fn set_keep_alive(mut self, secs: u16) -> Self {
        if secs < 10 {
            panic!("Keep alives should be >= 10 secs");
        }

        self.keep_alive = Some(Duration::from_secs(secs as u64));
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

    /// Set how to connect to a MQTT Broker (either plain TCP or SSL)
    pub fn set_connection_method(mut self, opts: ConnectionMethod) -> Self {
        self.connection_method = opts;
        self
    }

    /// Time interval after which client should retry for new
    /// connection if there are any disconnections. By default, no retry will happen
    pub fn set_reconnect_opts(mut self, opts: ReconnectOptions) -> Self {
        self.reconnect = opts;
        self
    }

    /// Set security option
    /// Supports username-password auth, tls client cert auth, gcloud iotcore jwt auth
    pub fn set_security_opts(mut self, opts: SecurityOptions) -> Self {
        self.security = opts;
        self
    }

    /// Set last will and testament
    pub fn set_last_will(mut self, last_will: LastWill) -> Self {
        self.last_will = Some(last_will);
        self
    }

    /// Clear last will and testament
    pub fn clear_last_will(mut self) -> Self {
        self.last_will = None;
        self
    }

    pub fn connect_packet(&self) -> Connect {
        let (username, password) = match self.security {
            SecurityOptions::UsernamePassword((ref username, ref password)) => (Some(username.to_owned()), Some(password.to_owned())),
            _ => (None, None),
        };
        let keep_alive: u16;
        match self.keep_alive {
            Some(ka) => keep_alive = ka.as_secs() as u16,
            _ => keep_alive = 60 as u16,
        };
        Connect {
            protocol: Protocol::MQTT(4),
            keep_alive: keep_alive,
            client_id: self.client_id.clone(),
            clean_session: self.clean_session,
            last_will: self.last_will.clone(),
            username: username,
            password: password,
        }
    }
}

#[cfg(test)]
mod test {
    use mqttoptions::{MqttOptions, ReconnectOptions};

    #[test]
    #[should_panic]
    fn client_id_startswith_space() {
        let _mqtt_opts = MqttOptions::new(" client_a", "127.0.0.1:1883")
            .set_reconnect_opts(ReconnectOptions::Always(10))
            .set_clean_session(true);
    }

    #[test]
    #[should_panic]
    fn no_client_id() {
        let _mqtt_opts = MqttOptions::new("", "127.0.0.1:1883")
            .set_reconnect_opts(ReconnectOptions::Always(10))
            .set_clean_session(true);
    }
}
