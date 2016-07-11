//! A fast, lock free Mqtt client implementation in Rust.
//!
//! **NOTE**: Though (almost) everything in the spec is working, this crate is
//! still in its early stages of development.
//! So please be aware of breakages. If you don't find any of the APIs elegant
//! or think that there is a
//! better way of implementation, please don't hesitate to raise an
//! issue/pullrequest.
//!
//! Below examples explain basic usage of this library
//!
//!
//! # Connecting to a broker
//!
//! ```ignore
//! let mut client_options = ClientOptions::new();
//!
//! //Specify client connection opthons and which broker to connect to
//! let proxy_client = client_options.set_keep_alive(5)
//!                                  .set_reconnect(5)
//!                                  .connect("localhost:1883");
//!
//! //Connects to a broker and returns a `Publisher` and `Subscriber`
//! let (publisher, subscriber) = proxy_client.start().expect("Coudn't start");
//! ```
//!
//!
//! # Publishing
//!
//! ```ignore
//! for i in 0..100 {
//!     let payload = format!("{}. hello rust", i);
//!     publisher.publish("hello/rust", QoS::Level1, payload.into_bytes());
//!     thread::sleep(Duration::new(1, 0));
//! }
//! ```
//!
//! # Subscribing
//!
//! ```ignore
//! let topics = vec![("hello/+/world", QoS::Level0),
//!                   ("hello/rust", QoS::Level1)];
//!
//! subscriber.message_callback(|message| {
//!        println!("@@@ {:?}", message);
//! });
//! subscriber.subscribe(topics).expect("Subscribe error");
//! ```
//!

extern crate rand;
#[macro_use]
extern crate log;
#[macro_use]
extern crate mioco;
extern crate mqtt;
#[macro_use]
extern crate chan;
extern crate time;
#[cfg(feature = "ssl")]
extern crate openssl;

mod error;
#[cfg(feature = "ssl")]
mod tls;
mod message;
mod client;

pub use client::ClientOptions;
pub use tls::SslContext;
pub use mqtt::QualityOfService as QoS;
