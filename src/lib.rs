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
//! use futures::stream::StreamExt;
//! use rumqtt::{MqttClient, MqttOptions, QoS};
//! use std::time::Duration;
//! use tokio::{spawn, time::delay_for};
//!
//! #[tokio::main]
//! async fn main() {
//!     let mqtt_options = MqttOptions::new("test-pubsub1", "localhost", 1883);
//!     let (mut mqtt_client, mut notifications) = MqttClient::start(mqtt_options).await.unwrap();
//!
//!     mqtt_client.subscribe("hello/world", QoS::AtLeastOnce).await.unwrap();
//!     let sleep_time = Duration::from_secs(1);
//!     spawn(async move {
//!         for i in 0..100 {
//!             let payload = format!("publish {}", i);
//!             delay_for(sleep_time).await;
//!             mqtt_client.publish("hello/world", QoS::AtLeastOnce, false, payload).await.unwrap();
//!         }
//!     });
//!
//!     while let Some(notification) = notifications.next().await {
//!         println!("{:?}", notification)
//!     }
//! }
//! ```
//!
//!
//! ## Select on incoming notifications using crossbeam select!
//! ```no_run
//! use futures::{channel::oneshot, select, FutureExt, StreamExt};
//! use rumqtt::{MqttClient, MqttOptions, QoS};
//! use std::time::Duration;
//! use tokio::{spawn, time::delay_for};
//!
//! #[tokio::main]
//! async fn main() {
//!     let mqtt_options = MqttOptions::new("test-pubsub1", "localhost", 1883);
//!     let (mut mqtt_client, mut notifications) = MqttClient::start(mqtt_options).await.unwrap();
//!     let (done_tx, done_rx) = oneshot::channel();
//!
//!     mqtt_client.subscribe("hello/world", QoS::AtLeastOnce).await.unwrap();
//!     let sleep_time = Duration::from_secs(1);
//!     spawn(async move {
//!         for i in 0..100 {
//!             let payload = format!("publish {}", i);
//!             delay_for(sleep_time).await;
//!             mqtt_client.publish("hello/world", QoS::AtLeastOnce, false, payload).await.unwrap();
//!         }
//!
//!         delay_for(sleep_time * 10).await;
//!         done_tx.send(()).unwrap();
//!     });
//!
//!     // select between mqtt notifications and other channel rx
//!     let mut done_rx = done_rx.fuse();
//!     loop {
//!         select! {
//!             notification = notifications.next() => {
//!                 println!("{:?}", notification)
//!             }
//!             res = done_rx => {
//!                 let () = res.unwrap();
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! ## Clone the client to access mqtt eventloop from different threads
//! ```no_run
//! use futures::stream::StreamExt;
//! use rumqtt::{MqttClient, MqttOptions, QoS};
//! use std::time::Duration;
//! use tokio::{spawn, time::delay_for};
//!
//! #[tokio::main]
//! async fn main() {
//!     let mqtt_options = MqttOptions::new("test-pubsub1", "localhost", 1883);
//!     let (mut mqtt_client, mut notifications) = MqttClient::start(mqtt_options).await.unwrap();
//!     let mut c1 = mqtt_client.clone();
//!     let mut c2 = mqtt_client.clone();
//!
//!     mqtt_client.subscribe("hello/world", QoS::AtLeastOnce).await.unwrap();
//!     let sleep_time = Duration::from_secs(1);
//!     spawn(async move {
//!         for i in 0..100 {
//!             let payload = format!("publish {}", i);
//!             delay_for(sleep_time).await;
//!             c1.publish("hello/world", QoS::AtLeastOnce, false, payload).await.unwrap();
//!         }
//!     });
//!
//!     // pause and resume network from this thread
//!     spawn(async move {
//!         let dur = Duration::new(5, 0);
//!         for i in 0..100 {
//!             if i % 2 == 0 { c2.pause().await.unwrap() } else { c2.resume().await.unwrap() }
//!             delay_for(dur).await;
//!         }
//!     });
//!
//!     // receive incoming notifications
//!     while let Some(notification) = notifications.next().await {
//!         println!("{:?}", notification)
//!     }
//! }
//! ```

#[macro_use]
extern crate log;
#[macro_use]
extern crate rental;

pub mod client;
pub mod codec;
pub mod error;
pub mod mqttoptions;

pub use crate::{
    client::{MqttClient, Notification},
    error::{ClientError, ConnectError},
    mqttoptions::{MqttOptions, Proxy, ReconnectOptions, SecurityOptions},
};
pub use futures::channel::mpsc::Receiver;
#[doc(hidden)]
pub use mqtt311::*;
