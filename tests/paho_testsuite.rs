extern crate rumqtt;
extern crate mqtt;
#[macro_use]
extern crate log;
extern crate env_logger;
#[cfg(feature = "ssl")]
extern crate openssl;
extern crate mioco;

use rumqtt::{ClientOptions, ReconnectMethod, SslContext};
use mqtt::{TopicFilter, QualityOfService};
use mioco::tcp::TcpStream;
use std::thread;
use std::time::Duration;
use std::net::ToSocketAddrs;
use openssl::ssl::{self, SslMethod};
use openssl::x509::X509FileType;
use std::net::SocketAddr;

#[test]
fn basic_test() {
    let mut client_options = ClientOptions::new();
    let proxy_client =client_options.connect("localhost:1883").expect("CONNECT ERROR");

    let (publisher, subscriber) = match proxy_client.await() {
        Ok(h) => h,
        Err(e) => panic!("Await Error --> {:?}", e),
    };

    let topics: Vec<(TopicFilter, QualityOfService)> =
    vec![(TopicFilter::new_checked("hello/world".to_string()).unwrap(),
              QualityOfService::Level2)];
    subscriber.subscribe(topics);
    thread::spawn(move || {
        thread::sleep(Duration::new(1, 0));
        //NOTE: TODO: Lot of clones. Change arg to &Vec<u8>. But cloning huge payload is bad
        let payload = format!("hello rust");
        publisher.publish("hello/world", QualityOfService::Level0, payload.clone().into_bytes());
        publisher.publish("hello/world", QualityOfService::Level1, payload.clone().into_bytes());
        publisher.publish("hello/world", QualityOfService::Level2, payload.clone().into_bytes());
        thread::sleep(Duration::new(1, 0));
    });

    let mut count = 0;
    for i in 0..3{
        count = i + 1;
        let message = subscriber.receive();
        println!("{:?}", message);
    }
    assert!(3 == count);
    thread::sleep(Duration::new(3, 0));
}

#[test]
fn retained_message_test() {
    let mut options = ClientOptions::new();
    options.set_client_id("client-1".to_string());
    let proxy_client = options.connect("localhost:1883").expect("CONNECT ERROR");
    let (mut publisher, subscriber) = proxy_client.await().expect("AWAIT ERROR");

    let payload = format!("hello rust");
    publisher.set_retain(true).publish("hello/world", QualityOfService::Level0, payload.clone().into_bytes());
    publisher.set_retain(true).publish("hello/world", QualityOfService::Level1, payload.clone().into_bytes());
    publisher.set_retain(true).publish("hello/world", QualityOfService::Level2, payload.clone().into_bytes());
    thread::sleep(Duration::new(1, 0));

    let topics = vec![(TopicFilter::new_checked("hello/world".to_string()).unwrap(),
              QualityOfService::Level2)];
    subscriber.subscribe(topics);
    let mut count = 0;
    for i in 0..3{
        count = i + 1;
        let message = subscriber.receive();
        println!("{:?}", message);
    }
    assert!(3 == count);
}