use mqtt3::Packet;

pub mod connection;
pub mod mqttstate;

pub type Notification = Option<Packet>;
pub type Reply = Option<Packet>;