extern crate tokio;
extern crate futures;
extern crate mqtt3;
extern crate tokio_io;
extern crate bytes;
#[macro_use]
extern crate log;
extern crate failure;

use tokio::runtime::current_thread;
use futures::sync::mpsc;

pub mod codec;

fn create_channel() {
    let (tx, rx) = mpsc::channel::<i32>(1);

    // let f = rx.for
}