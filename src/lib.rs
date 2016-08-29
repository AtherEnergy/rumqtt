
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
//! # Connecting to a broker
//!
//! ```
//! use rumqtt::{MqttOptions, MqttClient, QoS};
//! // Specify client connection options
//! let client_options = MqttOptions::new()
//!                                   .set_keep_alive(5)
//!                                   .set_reconnect(3)
//!                                   .set_client_id("rumqtt-docs")
//!                                   .broker("broker.hivemq.com:1883");
//!
//! // Create a new `MqttClient` object from `MqttOptions`
//! let mq_client = MqttClient::new(client_options);
//!
//! // Set callback for receiving incoming messages.
//! // This is optional if you don't want to subscribe
//! // to any topic.
//! let mq_client = mq_client.message_callback(move |message| {
//!     println!("message --> {:?}", message);
//! });
//!
//! // Connects to the broker, starts event loop and returns a `Request`
//! // object for making mqtt requests like `publish`, `subscribe` etc..
//! let request = mq_client.start().expect("Coudn't start");
//! ```
//!
//!
//! # Publishing
//!
//! ```ignore
//! let payload = format!("{}. hello rust", i);
//! request.publish("hello/rust", QoS::Level1, payload.into_bytes())
//!          .expect("Publish failure");
//! ```
//!
//! # Subscribing
//!
//! ```ignore
//! let topics = vec![("hello/+/world", QoS::Level0),
//!                   ("hello/rust", QoS::Level1)];
//!
//! request.subscribe(topics).expect("Subcription failure");
//! ```
//!

extern crate rand;
#[macro_use]
extern crate log;
#[macro_use]
extern crate mio;
extern crate mqtt;
extern crate time;
extern crate openssl;
extern crate threadpool;

mod error;
mod genpack;
mod tls;
mod message;
mod clientoptions;
mod request;
mod client;

pub use error::{Error, Result};
pub use clientoptions::MqttOptions;
pub use mqtt::QualityOfService as QoS;
pub use client::MqttClient;
pub use request::MqRequest;
pub use message::Message;
