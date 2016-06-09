extern crate rand;
extern crate mioco;

pub mod error;
pub mod client;

pub use client::{Client, ClientOptions};

use std::sync::Arc;
use std::ops;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientState {
    Handshake,
    Connected,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectMethod {
    ForeverDisconnect,
    ReconnectAfter(Duration),
}
