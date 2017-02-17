use std::sync::Arc;

use message::Message;

type MessageSendableFn = Box<Fn(Message) + Send + Sync>;
type PublishSendableFn = Box<Fn(Message) + Send + Sync>;

pub struct MqttCallback {
    pub on_message: Option<Arc<MessageSendableFn>>,
    pub on_publish: Option<Arc<PublishSendableFn>>,
}

impl MqttCallback {
    pub fn new() -> Self {
        MqttCallback {
            on_message: None,
            on_publish: None,
        }
    }

    pub fn on_message<F>(mut self, cb: F) -> Self
        where F: Fn(Message) + Sync + Send + 'static
    {
        self.on_message = Some(Arc::new(Box::new(cb)));
        self
    }

    pub fn on_publish<F>(mut self, cb: F) -> Self
        where F: Fn(Message) + Sync + Send + 'static
    {
        self.on_publish = Some(Arc::new(Box::new(cb)));
        self
    }
}
