extern crate bytes;
extern crate futures;
extern crate mqtt3;
extern crate tokio;
extern crate tokio_io;
extern crate tokio_codec;
#[macro_use]
extern crate log;
#[macro_use]
extern crate failure;

use futures::sync::mpsc;
use tokio::runtime::current_thread;

pub mod client;
pub mod codec;
pub mod mqttoptions;
pub mod error;

fn create_channel() {
    // let (tx, rx) = mpsc::channel::<i32>(1);

    // let f = rx.for
}
