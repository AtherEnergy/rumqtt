use super::client::MqttClient;
use std::sync::{Arc, Mutex};

impl MqttClient {
    pub fn on_message<F>(&mut self, callback: F) -> Result<&Self, i32>
        where F: Fn(&str, &str) + Send + Sync + 'static
    {
        let callback = Arc::new(Mutex::new(callback));
        self.msg_callback = Some(callback);
        Ok(self)
    }
}
