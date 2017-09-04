use rand::{self, Rng};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct MqttOptions {
    pub addr: String,
    pub keep_alive: Option<u16>,
    pub clean_session: bool,
    pub reconnect: u16,
    pub first_reconnection_loop: bool,
    pub client_id: Option<String>,
    pub credentials: Option<(String, String)>,
    pub ca: Option<PathBuf>,
    // root cert for encryption & private key for signing jwt token,
    // token expiry time
    pub googleiotcore_auth: Option<(PathBuf, i64)>,
    pub client_certs: Option<(PathBuf, PathBuf)>,
    pub storepack_sz: usize,
    pub await_batch_size: u32,
}

impl Default for MqttOptions {
    fn default() -> Self {
        MqttOptions {
            addr: "localhost:1883".to_string(),
            keep_alive: Some(10),
            clean_session: false,
            client_id: None,
            credentials: None,
            reconnect: 5,
            first_reconnection_loop: false,
            ca: None,
            googleiotcore_auth: None,
            client_certs: None,
            storepack_sz: 100 * 1024,
            await_batch_size: 10,
        }
    }
}

impl MqttOptions {
    /// Creates a new `MqttOptions` object which is used to set connection
    /// options for new client. Below are defaults with which this object is
    /// created.
    ///
    /// |                         |                          |
    /// |-------------------------|--------------------------|
    /// | **client_id**           | Randomly generated       |
    /// | **clean_session**       | false                    |
    /// | **keep_alive**          | 10 secs                  |
    /// | **reconnect try**       | Doesn't try to reconnect |
    /// | **retransmit try time** | 60 secs                  |
    /// | **pub_q_len**           | 50                       |
    /// | **sub_q_len**           | 5                        |
    ///
    pub fn new() -> MqttOptions {
        MqttOptions { ..Default::default() }
    }

    /// Number of seconds after which client should ping the broker
    /// if there is no other data exchange
    pub fn set_keep_alive(mut self, secs: u16) -> Self {
        if secs < 3 {
            panic!("Keep alives should be greater than 3 secs");
        } 

        self.keep_alive = Some(secs);
        self
    }

    /// Client id of the client. A random client id will be selected

    /// if you don't set one
    pub fn set_client_id(mut self, client_id: &str) -> Self {
        self.client_id = Some(client_id.to_string());
        self
    }

    /// Size limit for packet persistance in queues (in KB's)
    pub fn set_storepack_sz(mut self, sz: usize) -> Self {
        self.storepack_sz = sz * 1024;
        self
    }

    /// `clean_session = true` instructs the broker to clean all the client
    /// state when it disconnects. Note that it is broker which is discarding
    /// the client state. But this client will hold its queues and attemts to
    /// to retransmit when reconnection happens.  (TODO: Verify this)
    ///
    /// When set `false`, broker will hold the client state and performs pending
    /// operations on the client when reconnection with same `client_id`
    /// happens.
    ///
    /// Hence **make sure that you manually set `client_id` when
    /// `clean_session` is false**
    pub fn set_clean_session(mut self, clean_session: bool) -> Self {
        self.clean_session = clean_session;
        self
    }

    fn generate_client_id(&mut self) -> &mut Self {
        let mut rng = rand::thread_rng();
        let id = rng.gen::<u32>();
        self.client_id = Some(format!("mqttc_{}", id));
        self
    }

    /// Set `username` for broker to perform client authentication
    /// via `username` and `password`
    pub fn set_credentials(mut self, username: &str, password: &str) -> Self {
        self.credentials = Some((username.to_string(), password.to_string()));
        self
    }

    /// Time interval after which client should retry for new
    /// connection if there are any disconnections.
    /// By default, no retry will happen
    pub fn set_reconnect_interval(mut self, dur: u16) -> Self {
        self.reconnect = dur;
        self
    }

    pub fn set_first_reconnect_loop(mut self, reconnect: bool) -> Self {
        self.first_reconnection_loop = reconnect;
        self
    }

    pub fn set_ca<P>(mut self, cafile: P) -> Self
        where P: AsRef<Path>
    {
        self.ca = Some(cafile.as_ref().to_path_buf());
        self
    }

    pub fn googleiotcore_auth<P>(mut self, key: P, expiry: i64) -> Self
    where P: AsRef<Path> {
        self.googleiotcore_auth = Some((key.as_ref().to_path_buf(), expiry));
        self
    }

    /// Set client cert and key for server to do client authentication
    pub fn set_client_certs<P>(mut self, certfile: P, keyfile: P) -> Self
    where
        P: AsRef<Path>,
    {
        let c = certfile.as_ref().to_path_buf();
        let k = keyfile.as_ref().to_path_buf();
        self.client_certs = Some((c, k));
        self
    }

    /// Creates a new mqtt client with the broker address that you want
    /// to connect to. Along with connection details, this object holds
    /// all the state information of a connection.
    ///
    /// **NOTE**: This should be the final call of `MqttOptions` method
    /// chaining
    ///
    /// ```
    /// use cloudpubsub::MqttOptions;
    /// let client = MqttOptions::new()
    ///                           .set_keep_alive(5)
    ///                           .set_reconnect_interval(5)
    ///                           .set_client_id("my-client-id")
    ///                           .set_clean_session(true)
    ///                           .set_broker("localhost:1883");
    ///
    pub fn set_broker(mut self, addr: &str) -> Self {
        if self.client_id == None {
            self.generate_client_id();
        }
        self.addr = addr.to_string();
        self
    }
}
