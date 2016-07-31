use rand::{self, Rng};
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use mqtt::QualityOfService;

pub struct MqttOptions {
    pub addr: Option<SocketAddr>, // TODO: Use a default localhost here instead of option
    pub keep_alive: Option<u16>,
    pub clean_session: bool,
    pub client_id: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub reconnect: Option<u16>,
    pub will: Option<(String, String)>,
    pub will_qos: QualityOfService,
    pub will_retain: bool,
    pub pub_q_len: u16,
    pub sub_q_len: u16,
    pub queue_timeout: u16, // wait time for ack beyond which packet(publish/subscribe) will be resent
    pub tls: Option<PathBuf>,
}

impl Default for MqttOptions {
    fn default() -> Self {
        MqttOptions {
            addr: None,
            keep_alive: Some(5),
            clean_session: true,
            client_id: None,
            username: None,
            password: None,
            reconnect: None,
            will: None,
            will_qos: QualityOfService::Level0,
            will_retain: false,
            pub_q_len: 50,
            sub_q_len: 5,
            queue_timeout: 60,
            tls: None,
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
    /// | **clean_session**       | true                     |
    /// | **keep_alive**          | 5 secs                   |
    /// | **reconnect try**       | Doesn't try to reconnect |
    /// | **retransmit try time** | 60 secs                  |
    /// | **pub_q_len**           | 50                       |
    /// | **sub_q_len**           | 5                        |
    ///
    pub fn new() -> MqttOptions { MqttOptions { ..Default::default() } }

    /// Number of seconds after which client should ping the broker
    /// if there is no other data exchange
    pub fn set_keep_alive(mut self, secs: u16) -> Self {
        self.keep_alive = Some(secs);
        self
    }

    /// Client id of the client. A random client id will be selected

    /// if you don't set one
    pub fn set_client_id(mut self, client_id: &str) -> Self {
        self.client_id = Some(client_id.to_string());
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
    pub fn set_user_name(mut self, username: &str) -> Self {
        self.username = Some(username.to_string());
        self
    }

    /// Set `password` for broker to perform client authentication
    /// vis `username` and `password`
    pub fn set_password(mut self, password: &str) -> Self {
        self.password = Some(password.to_string());
        self
    }

    /// All the `QoS > 0` publishes state will be saved to attempt
    /// retransmits incase ack from broker fails.
    ///
    /// If broker disconnects for some time, `Publisher` shouldn't throw error
    /// immediately during publishes. At the same time, `Publisher` shouldn't be
    /// allowed to infinitely push to the queue.
    ///
    /// Publish queue length specifies maximum queue capacity upto which
    /// `Publisher`
    /// can push with out blocking. Messages in this queue will published as
    /// soon as
    /// connection is reestablished and `Publisher` gets unblocked
    pub fn set_pub_q_len(mut self, len: u16) -> Self {
        self.pub_q_len = len;
        self
    }

    pub fn set_sub_q_len(mut self, len: u16) -> Self {
        self.sub_q_len = len;
        self
    }

    /// Time interval after which client should retry for new
    /// connection if there are any disconnections.
    /// By default, no retry will happen
    // TODO: Rename
    pub fn set_reconnect(mut self, dur: u16) -> Self {
        self.reconnect = Some(dur);
        self
    }

    /// Set will for the client so that broker can send `will_message`
    /// on `will_topic` when this client ungracefully dies.
    pub fn set_will(mut self, will_topic: &str, will_message: &str) -> Self {
        self.will = Some((will_topic.to_string(), will_message.to_string()));
        self
    }

    /// Set QoS for the will message
    pub fn set_will_qos(mut self, qos: QualityOfService) -> Self {
        self.will_qos = qos;
        self
    }

    /// Set will retian so that future clients subscribing to will topic
    /// knows of client's death.
    pub fn set_will_retain(mut self, retain: bool) -> Self {
        self.will_retain = retain;
        self
    }

    /// Set a TLS connection
    pub fn set_tls<P>(mut self, cafile: P) -> Self
    where P: AsRef<Path> {
        self.tls = Some(cafile.as_ref().to_path_buf());
        self
    }

    fn lookup_ipv4<A: ToSocketAddrs>(addr: A) -> SocketAddr {
        let addrs = addr.to_socket_addrs().expect("Conversion Failed");
        for addr in addrs {
            if let SocketAddr::V4(_) = addr {
                return addr;
            }
        }
        unreachable!("Cannot lookup address");
    }

    /// Creates a new mqtt client with the broker address that you want
    /// to connect to. Along with connection details, this object holds
    /// all the state information of a connection.
    ///
    /// **NOTE**: This should be the final call of `MqttOptions` method
    /// chaining
    ///
    /// ```ignore
    /// let client = client_options.set_keep_alive(5)
    ///                           .set_reconnect(5)
    ///                           .set_client_id("my-client-id")
    ///                           .set_clean_session(true)
    ///                           .connect("localhost:1883");
    ///
    // TODO: Rename
    pub fn broker<A: ToSocketAddrs>(mut self, addr: A) -> Self {
        if self.client_id == None {
            self.generate_client_id();
        }
        self.addr = Some(Self::lookup_ipv4(addr));
        self
    }
}
