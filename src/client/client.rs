extern crate time;

use std::default::Default;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::Sender;
use std::net::TcpStream;
use std::collections::LinkedList;
use std::sync::atomic::AtomicUsize;

pub type SendableFn = Arc<Mutex<(Fn(&str, &str) + Send + Sync + 'static)>>;

#[derive(Debug)]
pub struct PublishMessage {
    pub pkid: u16,
    pub message: String,
    pub timestamp: i64,
}

pub struct MqttConnectionOptions {
    pub id: String,
    pub keep_alive: u16,
    pub clean_session: bool,
}

pub struct PublishQueue {
    pub queue: Arc<Mutex<LinkedList<PublishMessage>>>, // Queue for QoS 1 & 2
    pub length: u16,
    pub current_pkid: AtomicUsize,
    pub retry_time: u16,
}

pub struct MqttClient {
    pub options: MqttConnectionOptions,
    pub stream: Option<TcpStream>,
    pub msg_callback: Option<Sender<SendableFn>>,
    pub publish_queue: PublishQueue,
}

impl Default for MqttClient {
    fn default() -> MqttClient {
        MqttClient {
            options: MqttConnectionOptions {
                id: "".to_string(),
                keep_alive: 0,
                clean_session: true,
            },
            stream: None,
            msg_callback: None,
            publish_queue: PublishQueue {
                queue: Arc::new(Mutex::new(LinkedList::new())),
                length: 500,
                current_pkid: AtomicUsize::new(1),
                retry_time: 60,
            },
        }
    }
}

impl MqttClient {
    pub fn new(id: &str) -> MqttClient {
        let mut client = MqttClient { ..Default::default() };
        client.options.id = id.to_string();
        client
    }

    // TODO: Implement keep_alive in lower layers
    pub fn keep_alive(mut self, val: u16) -> Self {
        self.options.keep_alive = val;
        self
    }

    pub fn clean_session(mut self, val: bool) -> Self {
        self.options.clean_session = val;
        self
    }
}
