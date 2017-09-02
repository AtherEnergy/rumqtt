#![feature(proc_macro, conservative_impl_trait, generators)]

extern crate futures_await as futures;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_timer;
extern crate mqtt3;
extern crate bytes;
#[macro_use]
extern crate quick_error;
#[macro_use]
extern crate log;
extern crate threadpool;

mod codec;
mod packet;
mod mqttopts;
mod client;
mod error;

// expose to other crates
pub use mqttopts::MqttOptions;
pub use client::MqttClient;
pub use mqtt3::QoS;