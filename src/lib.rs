#![warn(missing_docs)]
//! Rumqtt is a pure rust mqtt client which strives to be robust
//! efficient and easy to use. 
//! * Provides several reconnection options to automate reconnections
//! * All the network requests are done using channels and bad networks can
//!   be detected through back pressure
//! * Incoming notifications are delivered to the user through crossbeam channel
//!   which provides a flexible `select!` macro [see select.rs](examples/select.rs)
#[macro_use]
extern crate log;

/// Provides apis to interact with mqtt eventloop thread
pub mod client;
/// Provides apis to configure mqtt connection
pub mod mqttoptions;
mod codec;
mod error;

pub use crate::client::{MqttClient, Notification};
pub use crate::mqttoptions::{ConnectionMethod, MqttOptions, Proxy, ReconnectOptions, SecurityOptions};
pub use crate::error::{ConnectError, ClientError};
pub use crossbeam_channel::Receiver;
pub use mqtt311::*;