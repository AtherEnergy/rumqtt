#![cfg_attr(feature="clippy", feature(plugin))]

#![cfg_attr(feature="clippy", plugin(clippy))]


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
//! use rumqtt::{MqttCallback};
//! use std::sync::mpsc;
//! // Specify client connection options
//! let client_options = MqttOptions::new()
//!                                   .set_keep_alive(5)
//!                                   .set_reconnect(3)
//!                                   .set_client_id("rumqtt-docs")
//!                                   .set_broker("test.mosquitto.org:1883");
//! // Create a new `MqttClient` object from `MqttOptions`
//!
//! // Set callback for receiving incoming messages.
//! let callback = |msg| {
//! 	println!("Received payload: {:?}", msg);
//! };
//! let mut mq_cbs = MqttCallback::new().on_message(callback);
//!
//!
//! // This is optional if you don't want to subscribe
//! // to any topic.
//!
//! // Connects to the broker, starts event loop and returns a `Request`
//! // object for making mqtt requests like `publish`, `subscribe` etc..
//! let request = MqttClient::start(client_options,
//! Some(mq_cbs)).expect("Coudn't start");
//! ```
//!
//!
//! # Publishing
//!
//!
//! // use rumqtt::{MqttOptions, MqttClient, QoS};
//! let payload = format!("{}. hello rust", 1);
//! request.publish("hello/rust", QoS::Level1, payload.into_bytes())
//!          .expect("Publish failure");
//!
//!
//! # Subscribing
//!
//!
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
extern crate quick_error;
extern crate mqtt;
extern crate time;
extern crate openssl;
extern crate threadpool;

mod error;
mod genpack;
mod stream;
mod message;
mod clientoptions;
mod connection;
mod client;
mod callbacks;

pub use error::{Error, Result};
pub use clientoptions::MqttOptions;
pub use mqtt::QualityOfService as QoS;
pub use client::MqttClient;
pub use message::Message;
pub use callbacks::MqttCallback;
