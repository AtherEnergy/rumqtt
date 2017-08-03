extern crate rand;
extern crate mqtt311;
#[macro_use]
extern crate log;
#[macro_use]
extern crate quick_error;
extern crate openssl;
extern crate threadpool;

pub mod error;
pub mod stream;
pub mod clientoptions;
pub mod callback;
pub mod publisher;
pub mod client;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttState {
    Handshake,
    Connected,
    Disconnected,
}

pub use clientoptions::MqttOptions;
pub use client::MqttClient;
pub use callback::{MqttCallback, Message};
pub use error::Error;
