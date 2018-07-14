extern crate bytes;
extern crate futures;
extern crate mqtt3;
extern crate tokio;
extern crate tokio_io;
#[macro_use]
extern crate log;
extern crate failure;

use futures::sync::mpsc;
use tokio::runtime::current_thread;

pub mod client;
pub mod codec;

fn create_channel() {
    let (tx, rx) = mpsc::channel::<i32>(1);

    // let f = rx.for
}
