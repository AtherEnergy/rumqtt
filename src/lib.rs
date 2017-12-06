extern crate futures;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_timer;
extern crate mqtt3;
extern crate bytes;
extern crate chrono;
extern crate jsonwebtoken as jwt;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate quick_error;
#[macro_use]
extern crate log;

mod codec;
mod packet;
mod mqttopts;
mod client;
mod error;

// expose to other crates
pub use mqttopts::{MqttOptions, ReconnectOptions, SecurityOptions};
pub use client::MqttClient;
pub use mqtt3::QoS;