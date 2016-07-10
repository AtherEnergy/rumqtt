/*!

An elegant, lock free Mqtt client implementation in Rust.

**NOTE**: Though (almost)everything is working, this crate is still in its early stages of development.
So please be aware of breakages. If you don't find any of they APIs elegant and think that there is a 
better way, please raise an issue/pullrequest.

Below examples explain basic usage of this library

# Connecting to a broker

```
let mut client_options = ClientOptions::new();

// Specify client connection opthons and which broker to connect to
let proxy_client = client_options.set_keep_alive(5);
                                    .set_reconnect(ReconnectMethod::ReconnectAfter(Duration::new(5,0)))
                                    .connect("localhost:1883");

// Connects to a broker and returns a `Publisher` and `Subscriber`
let (publisher, subscriber) = proxy_client.start().expect("Coudn't start");
```


# Publishing

```
for i in 0..100 {
    let payload = format!("{}. hello rust", i);
    publisher.publish("hello/rust", QualityOfService::Level1, payload.into_bytes());
    thread::sleep(Duration::new(1, 0));
}
```

# Subscribing

TODO: Doesn't look great. Refine this.
TODO: Expose name QualityOfService as QoS
```
let topics = vec![(TopicFilter::new_checked("hello/world".to_string()).unwrap(),
                                                        QualityOfService::Level0)];
subscriber.subscribe(topics, 
                    |message| {
                        println!("@@@ {:?}", message);
                    })
                    .expect("Subscribe error");
```
*/

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
pub mod client;

pub use client::{ClientOptions, ReconnectMethod};
pub use tls::SslContext;