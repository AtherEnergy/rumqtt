use mqtt3::{Connect, LastWill, Protocol};

use error::ConnectError;
use std::time::Duration;

/// Control how the connection is re-established if it is lost.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ReconnectOptions {
    /// Don't automatically reconnect
    Never,
    /// Reconnect automatically if the connection has been established
    /// successfully at least once.
    ///
    /// Before a reconnection attempt, sleep for the specified amount of seconds.
    AfterFirstSuccess(u64),
    /// Always reconnect automatically.
    ///
    /// Before a reconnection attempt, sleep for the specified amount of seconds.
    Always(u64),
}

/// Client authentication option for mqtt connect packet
#[derive(Clone, Debug)]
pub enum SecurityOptions {
    /// No authentication.
    None,
    /// Use the specified `(username, password)` tuple to authenticate.
    UsernamePassword((String, String)),
    #[cfg(feature = "jwt")]
    /// Authenticate against a Google Cloud IoT Core project with the triple
    /// `(project name, private_key.der to sign jwt, expiry in seconds)`.
    GcloudIot((String, Vec<u8>, i64)),
}

#[derive(Clone, Debug)]
pub enum ConnectionMethod {
    Tcp,
    // ca and, optionally, a pair of client cert and client key
    Tls(Vec<u8>, Option<(Vec<u8>, Vec<u8>)>),
}


#[derive(Clone, Debug)]
pub struct MqttOptions {
    /// broker address that you want to connect to
    broker_addr: String,
    port: u16,
    /// keep alive time to send pingreq to broker when the connection is idle
    keep_alive: Duration,
    /// clean (or) persistent session
    clean_session: bool,
    /// client identifier
    client_id: String,
    /// connection method
    connection_method: ConnectionMethod,
    /// reconnection options
    reconnect: ReconnectOptions,
    /// security options
    security: SecurityOptions,
    /// maximum packet size
    max_packet_size: usize,
    /// last will and testament
    last_will: Option<LastWill>,
}

impl Default for MqttOptions {
    fn default() -> Self {
        MqttOptions { broker_addr: "127.0.0.1".into(),
                      port: 1883,
                      keep_alive: Duration::from_secs(30),
                      clean_session: true,
                      client_id: "test-client".into(),
                      connection_method: ConnectionMethod::Tcp,
                      reconnect: ReconnectOptions::AfterFirstSuccess(10),
                      security: SecurityOptions::None,
                      max_packet_size: 256 * 1024,
                      last_will: None }
    }
}

impl MqttOptions {
    pub fn new<S: Into<String>, T: Into<String>>(id: S, host: T, port: u16) -> MqttOptions {
        // TODO: Validate if addr is proper address type
        let id = id.into();
        if id.starts_with(' ') || id.is_empty() {
            panic!("Invalid client id")
        }

        MqttOptions { broker_addr: host.into(),
                      port,
                      keep_alive: Duration::from_secs(60),
                      clean_session: true,
                      client_id: id,
                      connection_method: ConnectionMethod::Tcp,
                      reconnect: ReconnectOptions::AfterFirstSuccess(10),
                      security: SecurityOptions::None,
                      max_packet_size: 256 * 1024,
                      last_will: None }
    }

    pub fn broker_address(&self) -> (String, u16) {
        (self.broker_addr.clone(), self.port)
    }

    /// Set number of seconds after which client should ping the broker
    /// if there is no other data exchange
    pub fn set_keep_alive(mut self, secs: u16) -> Self {
        if secs < 10 {
            panic!("Keep alives should be >= 10 secs");
        }

        self.keep_alive = Duration::from_secs(u64::from(secs));
        self
    }

    pub fn keep_alive(&self) -> Duration {
        self.keep_alive
    }

    /// Set packet size limit (in Kilo Bytes)
    pub fn set_max_packet_size(mut self, sz: usize) -> Self {
        self.max_packet_size = sz * 1024;
        self
    }

    pub fn max_packet_size(&self) -> usize {
        self.max_packet_size
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

    pub fn clean_session(&self) -> bool {
        self.clean_session
    }

    /// Set how to connect to a MQTT Broker (either plain TCP or SSL)
    pub fn set_connection_method(mut self, opts: ConnectionMethod) -> Self {
        self.connection_method = opts;
        self
    }

    pub fn connection_method(&self) -> ConnectionMethod {
        self.connection_method.clone()
    }

    /// Time interval after which client should retry for new
    /// connection if there are any disconnections. By default, no retry will happen
    pub fn set_reconnect_opts(mut self, opts: ReconnectOptions) -> Self {
        self.reconnect = opts;
        self
    }

    pub fn reconnect_opts(&self) -> ReconnectOptions {
        self.reconnect
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

    pub fn connect_packet(&self) -> Result<Connect, ConnectError> {
        let (username, password) = match self.security.clone() {
            SecurityOptions::UsernamePassword((username, password)) => (Some(username), Some(password)),
            #[cfg(feature = "jwt")]
            SecurityOptions::GcloudIot((projectname, key, expiry)) => {
                let username = Some("unused".to_owned());
                let password = Some(gen_iotcore_password(projectname, &key, expiry)?);
                (username, password)
            }
            SecurityOptions::None => (None, None),
        };

        let connect = Connect { protocol: Protocol::MQTT(4),
                                keep_alive: self.keep_alive.as_secs() as u16,
                                client_id: self.client_id.clone(),
                                clean_session: self.clean_session,
                                last_will: self.last_will.clone(),
                                username,
                                password };

        Ok(connect)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    iat: i64,
    exp: i64,
    aud: String,
}

#[cfg(feature = "jwt")]
// Generates a new password for mqtt client authentication
pub fn gen_iotcore_password(project: String, key: &[u8], expiry: i64) -> Result<String, ConnectError> {
    use chrono::{self, Utc};
    use jsonwebtoken::{encode, Algorithm, Header};

    let time = Utc::now();
    let jwt_header = Header::new(Algorithm::RS256);
    let iat = time.timestamp();
    let exp = time.checked_add_signed(chrono::Duration::minutes(expiry))
                  .expect("Unable to create expiry")
                  .timestamp();

    let claims = Claims { iat, exp, aud: project };

    Ok(encode(&jwt_header, &claims, &key)?)
}

#[cfg(test)]
mod test {
    use mqttoptions::{MqttOptions, ReconnectOptions};

    #[test]
    #[should_panic]
    fn client_id_startswith_space() {
        let _mqtt_opts = MqttOptions::new(" client_a", "127.0.0.1", 1883).set_reconnect_opts(ReconnectOptions::Always(10))
                                                                         .set_clean_session(true);
    }

    #[test]
    #[should_panic]
    fn no_client_id() {
        let _mqtt_opts = MqttOptions::new("", "127.0.0.1", 1883).set_reconnect_opts(ReconnectOptions::Always(10))
                                                                .set_clean_session(true);
    }
}
