extern crate bytes;
#[cfg(feature = "jwt")]
extern crate chrono;
extern crate crossbeam_channel;
extern crate futures;
#[cfg(feature = "jwt")]
extern crate jsonwebtoken;
extern crate mqtt311;
#[cfg(feature = "nativetls")]
extern crate native_tls;
extern crate tokio;
extern crate tokio_codec;
extern crate tokio_io;
#[cfg(feature = "rustls")]
extern crate tokio_rustls;
extern crate tokio_timer;
#[cfg(feature = "nativetls")]
extern crate tokio_tls;
#[cfg(feature = "rustls")]
extern crate webpki;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate derive_more;
#[macro_use]
extern crate log;
#[macro_use]
extern crate failure;
extern crate core;
extern crate pretty_env_logger;

pub mod client;
pub mod codec;
pub mod error;
pub mod mqttoptions;

pub use client::MqttClient;
pub use mqtt311::{QoS, PacketIdentifier};
pub use mqttoptions::{ConnectionMethod, MqttOptions, ReconnectOptions, SecurityOptions};
pub use crossbeam_channel::Receiver;
