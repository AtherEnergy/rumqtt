extern crate time;

use std::default::Default;
use std::sync::{Arc, Mutex};
use std::net::TcpStream;
use std::collections::LinkedList;
use std::sync::atomic::AtomicUsize;

pub type SendableFn = Arc<Mutex<(Fn(&str, &str) + Send + Sync + 'static)>>;

#[derive(Debug)]
pub struct PublishMessage {
    pub pkid: u16,
    pub topic: String,
    pub message: String,
    pub timestamp: i64,
}


#[derive(Clone)]
pub struct MqttConnectionOptions {
    pub id: String,
    pub keep_alive: u16,
    pub clean_session: bool,
}

pub struct MqttConnection {
    pub stream: Option<TcpStream>,
    pub current_pkid: AtomicUsize,
    pub queue: LinkedList<PublishMessage>, // Queue for QoS 1 & 2
    pub length: u16,
    pub retry_time: u16,
    pub host: String,
    pub options: MqttConnectionOptions,
}

#[derive(Clone)]
pub struct MqttClient {
    pub connection: Arc<Mutex<MqttConnection>>,
    pub msg_callback: Option<SendableFn>,
}

impl Default for MqttConnectionOptions {
    fn default() -> MqttConnectionOptions {
        MqttConnectionOptions {
            id: "".to_string(),
            keep_alive: 0,
            clean_session: true,
        }
    }
}

impl Default for MqttConnection {
    fn default() -> MqttConnection {
        MqttConnection {
            stream: None,
            queue: LinkedList::new(),
            length: 500,
            current_pkid: AtomicUsize::new(1),
            retry_time: 10,
            host: "".to_string(),
            options: MqttConnectionOptions { ..Default::default() },
        }
    }
}

impl Default for MqttClient {
    fn default() -> MqttClient {
        MqttClient {
            // thread safe connection
            connection: Arc::new(Mutex::new(MqttConnection { ..Default::default() })),
            // thread safe callback
            msg_callback: None,
        }
    }
}

impl MqttConnectionOptions {
    pub fn new(id: &str) -> MqttConnectionOptions {
        let mut options = MqttConnectionOptions { ..Default::default() };
        options.id = id.to_string();
        options
    }

    // TODO: Implement keep_alive in lower layers
    pub fn keep_alive(mut self, val: u16) -> Self {
        self.keep_alive = val;
        self
    }

    pub fn clean_session(mut self, val: bool) -> Self {
        self.clean_session = val;
        self
    }

    pub fn create_client(self) -> MqttClient {
        MqttClient {
            connection: Arc::new(Mutex::new(MqttConnection {
                options: self,
                ..Default::default()
            })),
            msg_callback: None,
        }
    }
}
