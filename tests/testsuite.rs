extern crate rumqtt;

use rumqtt::{ClientOptions, SslContext, QoS};
use std::thread;
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};


#[test]
fn basic_test() {
    let mut client_options = ClientOptions::new();

    // Specify client connection opthons and which broker to connect to
    let proxy_client = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .connect("test.mosquitto.org:1883");

    // Connects to a broker and returns a `Publisher` and `Subscriber`
    let (publisher, subscriber) = proxy_client.start().expect("Coudn't start");
    
    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let topics = vec![("hello/rust", QoS::Level0)];

    subscriber.subscribe(topics, move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        println!("message --> {:?}", message);
    }).expect("Subcription failure");
    
    let payload = format!("hello rust");
    publisher.publish("hello/rust", QoS::Level0, payload.clone().into_bytes());
    publisher.publish("hello/rust", QoS::Level1, payload.clone().into_bytes());
    publisher.publish("hello/rust", QoS::Level2, payload.clone().into_bytes());

    thread::sleep(Duration::new(3, 0));
    assert!(3 == final_count.load(Ordering::SeqCst));
    thread::sleep(Duration::new(3, 0));
}

#[test]
fn retained_message_test() {
    let mut client_options = ClientOptions::new();
    let proxy_client = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .set_client_id("client-1")
                                    .connect("test.mosquitto.org:1883");

    let (mut publisher, subscriber) = proxy_client.start().expect("Coudn't start");

    let payload = format!("hello rust");
    publisher.set_retain(true).publish("hello/1/world", QoS::Level0, payload.clone().into_bytes());
    publisher.set_retain(true).publish("hello/2/world", QoS::Level1, payload.clone().into_bytes());
    publisher.set_retain(true).publish("hello/3/world", QoS::Level2, payload.clone().into_bytes());

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let topics = vec![("hello/world", QoS::Level0)];
    subscriber.subscribe(topics, move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        println!("message --> {:?}", message);
    }).expect("Subcription failure");

    thread::sleep(Duration::new(30, 0));
    assert!(3 == final_count.load(Ordering::SeqCst));
    thread::sleep(Duration::new(3, 0));
}