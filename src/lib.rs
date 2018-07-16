extern crate bytes;
extern crate futures;
extern crate mqtt3;
extern crate tokio;
extern crate tokio_codec;
extern crate tokio_io;
extern crate tokio_current_thread;
extern crate tokio_core;
extern crate tokio_timer;
#[macro_use]
extern crate log;
#[macro_use]
extern crate failure;
extern crate pretty_env_logger;

use futures::sync::mpsc;
use tokio::runtime::current_thread;

pub mod client;
pub mod codec;
pub mod error;
pub mod mqttoptions;
pub mod mqttstate;

fn create_channel() {
    // let (tx, rx) = mpsc::channel::<i32>(1);

    // let f = rx.for
}
