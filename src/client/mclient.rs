use std::default::Default;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::Sender;
use std::net::TcpStream;

pub type SendableFn = Arc<Mutex<(Fn(&str, &str) + Send + Sync + 'static)>>;

pub struct MqttClient {
    pub id: String,
    pub keep_alive: i32,
    pub clean_session: bool,
    pub stream: Option<TcpStream>,
    pub tx: Option<Sender<SendableFn>>,
}

impl Default for MqttClient {
    fn default() -> MqttClient {
        MqttClient {
            id: "".to_string(),
            keep_alive: 3,
            clean_session: true,
            stream: None,
            tx: None,
        }
    }
}

impl MqttClient {
    pub fn new(id: &str) -> MqttClient {
        let client = MqttClient { id: id.to_string(), ..Default::default() };
        client
    }

    // TODO: Implement keep_alive in lower layers
    pub fn keep_alive(mut self, val: i32) -> Self {
        self.keep_alive = val;
        self
    }

    pub fn clean_session(mut self, val: bool) -> Self {
        self.clean_session = val;
        self
    }
}
