//! Rumqtt is a pure rust mqtt client which strives to be robust, efficient
//! and easy to use.
//! * Provides several reconnection options to automate reconnections
//! * All the network requests are done using channels and bad networks can
//!   be detected through back pressure
//! * Incoming notifications are delivered to the user through crossbeam channel
//!   which provides a flexible `select!` macro
//! * Clone the client to access mqtt eventloop from multiple threads
//! * Dynamically start and stop the network eventloop (Useful when you want the other network services to have more bandwidth)
//! * Inbuilt support for connecting to gcloud iot core which uses jwt tokens
//!   as password to authenticate the client
//! * Inbuilt support fot throttling (Saas IOT providers usually impose rate limiting)
//! * Http `connect` support to tunnel mqtt data through http proxy servers
//! 
//! ## Publish and subscribe
//! ```no_run
//! use rumqtt::{MqttClient, MqttOptions, QoS};
//! use std::{thread, time::Duration};
//!
//! fn main() {
//!     let mqtt_options = MqttOptions::new("test-pubsub1", "localhost", 1883);
//!     let (mut mqtt_client, notifications) = MqttClient::start(mqtt_options).unwrap();
//!      
//!     mqtt_client.subscribe("hello/world", QoS::AtLeastOnce).unwrap();
//!     let sleep_time = Duration::from_secs(1);
//!     thread::spawn(move || {
//!         for i in 0..100 {
//!             let payload = format!("publish {}", i);
//!             thread::sleep(sleep_time);
//!             mqtt_client.publish("hello/world", QoS::AtLeastOnce, false, payload).unwrap();
//!         }
//!     });
//!
//!     for notification in notifications {
//!         println!("{:?}", notification)
//!     }
//! }
//! ```
//! 
//! 
//! ## Select on incoming notifications using crossbeam select!
//! ```no_run
//! use rumqtt::{MqttClient, MqttOptions, QoS};
//! use std::{thread, time::Duration};
//! use crossbeam_channel::select;
//!
//! fn main() {
//!     let mqtt_options = MqttOptions::new("test-pubsub1", "localhost", 1883);
//!     let (mut mqtt_client, notifications) = MqttClient::start(mqtt_options).unwrap();
//!     let (done_tx, done_rx) = crossbeam_channel::bounded(1);
//! 
//!     mqtt_client.subscribe("hello/world", QoS::AtLeastOnce).unwrap();
//!     let sleep_time = Duration::from_secs(1);
//!     thread::spawn(move || {
//!         for i in 0..100 {
//!             let payload = format!("publish {}", i);
//!             thread::sleep(sleep_time);
//!             mqtt_client.publish("hello/world", QoS::AtLeastOnce, false, payload).unwrap();
//!         }
//!         
//!         thread::sleep(sleep_time * 10);
//!         done_tx.send(true).unwrap();
//!     });
//!
//!     // select between mqtt notifications and other channel rx
//!     loop {
//!         select! {
//!             recv(notifications) -> notification => {
//!                 println!("{:?}", notification)
//!             }
//!             recv(done_rx) -> _done => break
//!         }
//!     }
//! }
//! ```
//! 
//! ## Clone the client to access mqtt eventloop from different threads
//! ```no_run
//! use rumqtt::{MqttClient, MqttOptions, QoS};
//! use std::{thread, time::Duration};
//!
//! fn main() {
//!     let mqtt_options = MqttOptions::new("test-pubsub1", "localhost", 1883);
//!     let (mut mqtt_client, notifications) = MqttClient::start(mqtt_options).unwrap();
//!     let mut c1 = mqtt_client.clone();
//!     let mut c2 = mqtt_client.clone();
//!     
//!     mqtt_client.subscribe("hello/world", QoS::AtLeastOnce).unwrap();
//!     let sleep_time = Duration::from_secs(1);
//!     thread::spawn(move || {
//!         for i in 0..100 {
//!             let payload = format!("publish {}", i);
//!             thread::sleep(sleep_time);
//!             c1.publish("hello/world", QoS::AtLeastOnce, false, payload).unwrap();
//!         }
//!     });
//! 
//!     // pause and resume network from this thread
//!     thread::spawn(move || {
//!         let dur = Duration::new(5, 0);
//!         for i in 0..100 {
//!             if i % 2 == 0 { c2.pause().unwrap() } else { c2.resume().unwrap() }
//!             thread::sleep(dur);
//!         }
//!     });
//!
//!     // receive incoming notifications
//!     for notification in notifications {
//!         println!("{:?}", notification)
//!     }
//! }
//! ```

#[macro_use]
extern crate log;

pub mod client;
pub mod codec;
pub mod error;
pub mod mqttoptions;

pub use crate::client::{MqttClient, Notification, Notifications};
pub use crate::mqttoptions::{ConnectionMethod, MqttOptions, Proxy, ReconnectOptions, SecurityOptions};
pub use crate::error::{ConnectError, ClientError};
pub use crossbeam_channel::Receiver;
#[doc(hidden)]
pub use mqtt311::*;
