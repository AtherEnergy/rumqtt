use mqtt3::Packet;
use std::sync::Arc;

pub mod connection;
pub mod mqttstate;

pub enum Notification {
    Publish(Arc<Vec<u8>>),
    PubAck(u16),
    PubRec(u16),
    PubRel(u16),
    PubComp(u16),
    SubAck(u16),
    Disconnect,
    None
}

pub enum Reply {
    PubAck(u16),
    None
}
