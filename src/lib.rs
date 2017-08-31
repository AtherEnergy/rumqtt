#![feature(proc_macro, conservative_impl_trait, generators)]

extern crate futures_await as futures;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_timer;
extern crate mqtt3;
extern crate bytes;
#[macro_use]
extern crate log;
extern crate threadpool;

mod codec;
mod packet;
mod mqttopts;
mod client;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::io::{self, ErrorKind};
use std::error::Error;
use std::time::Duration;
use std::thread;

// expose to other crates
pub use mqttopts::MqttOptions;