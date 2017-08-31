#![feature(proc_macro, conservative_impl_trait, generators)]

extern crate futures_await as futures;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_timer;
extern crate mqtt3;
extern crate bytes;
#[macro_use]
extern crate log;

mod codec;
mod packet;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::io::{self, ErrorKind};
use std::error::Error;
use std::time::Duration;
use std::thread;

use codec::MqttCodec;

use futures::prelude::*;
use futures::stream::{Stream, SplitSink};
use futures::sync::mpsc::{self, Sender, Receiver};
use std::sync::mpsc as stdmpsc;
use tokio_core::reactor::Core;
use tokio_core::net::TcpStream;
use tokio_timer::Timer;
use tokio_io::AsyncRead;
use tokio_io::codec::Framed;
use mqtt3::*;