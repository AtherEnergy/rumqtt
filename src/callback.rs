use std::sync::Arc;
use mqtt311;

#[derive(Debug, Clone)]
pub struct Message {
    pub topic: String,
    pub payload: Arc<Vec<u8>>,
    pub pkid: Option<mqtt311::PacketIdentifier>,
    pub userdata: Option<Arc<Vec<u8>>>,
}

impl Message {
    // pub fn from_pub(publish: Box<mqtt311::Publish>) -> Result<Box<Message>> {
    //     let m = mqtt311::Message::from_pub(publish)?;
    //     let message = Message {
    //         message: *m,
    //         userdata: None,
    //     };
    //     Ok(Box::new(message))
    // }

    pub fn set_pkid(mut self, pkid: Option<mqtt311::PacketIdentifier>) -> Box<Message> {
        self.pkid = pkid;
        Box::new(self)
    }

    pub fn to_mqtt_message(&self) -> Box<mqtt311::Message> {
        Box::new(mqtt311::Message {
            topic: mqtt311::TopicPath::from_str(self.topic.to_string()).expect("Invalid Topic"),
            retain: false,
            qos: mqtt311::QoS::AtLeastOnce,
            payload: self.payload.clone(),
            pid: self.pkid,
        })
    }
}

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
    where
        F: Fn(Message) + Sync + Send + 'static,
    {
        self.on_message = Some(Arc::new(Box::new(cb)));
        self
    }

    pub fn on_publish<F>(mut self, cb: F) -> Self
    where
        F: Fn(Message) + Sync + Send + 'static,
    {
        self.on_publish = Some(Arc::new(Box::new(cb)));
        self
    }
}
