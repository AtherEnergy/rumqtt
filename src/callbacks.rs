use message::Message;

pub type MessageSendableFn = Box<Fn(Message) + Send + Sync>;
pub type PublishSendableFn = Box<Fn(Message) + Send + Sync>;

pub struct MqttCallback {
    on_message: Option<MessageSendableFn>,
    on_publish: Option<PublishSendableFn>
}

impl MqttCallback {
    pub fn new() -> Self {
        MqttCallback {
            on_message: None,
            on_publish: None
        }
    }

    pub fn on_message(mut self, cb: MessageSendableFn) -> Self {
        self.on_message = Some(cb);
        self
    }

     pub fn on_publish(mut self, cb: PublishSendableFn) -> Self {
        self.on_publish = Some(cb);
        self
    }
}