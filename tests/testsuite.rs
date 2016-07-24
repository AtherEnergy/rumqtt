extern crate rumqtt;

use rumqtt::{MqttOptions, QoS};
use std::thread;
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
#[macro_use]
extern crate log;
extern crate env_logger;


#[test]
fn basic() {
    env_logger::init().unwrap();
    let mut client_options = MqttOptions::new();

    // Specify client connection opthons and which broker to connect to
    let proxy_client = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .connect("broker.hivemq.com:1883");

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    // Connects to a broker and returns a `Publisher` and `Subscriber`
    let (publisher, subscriber) = proxy_client.
    message_callback(move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        println!("message --> {:?}", message);
    }).start().expect("Coudn't start");

    let topics = vec![("test/basic", QoS::Level0)];

    subscriber.subscribe(topics).expect("Subcription failure");  
    
    let payload = format!("hello rust");
    publisher.publish("test/basic", QoS::Level0, payload.clone().into_bytes()).unwrap();
    publisher.publish("test/basic", QoS::Level1, payload.clone().into_bytes()).unwrap();
    //publisher.publish("test/basic", QoS::Level2, payload.clone().into_bytes()).unwrap();

    thread::sleep(Duration::new(1, 0));
    assert!(2 == final_count.load(Ordering::SeqCst));
}

#[test]
fn reconnection() {
    // Create client in clean_session = false for broker to
    // remember your subscription after disconnect.
    let mut client_options = MqttOptions::new();
    let proxy_client = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .set_client_id("test-reconnect-client")
                                    .set_clean_session(false)
                                    .connect("broker.hivemq.com:1883");

    // Message count
    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    // Connects to a broker and returns a `Publisher` and `Subscriber`
    let (publisher, subscriber) = proxy_client.
    message_callback(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
        // println!("message --> {:?}", message);
    }).start().expect("Coudn't start");

    // Register message callback and subscribe
    let topics = vec![("test/reconnect", QoS::Level2)];  
    subscriber.subscribe(topics).expect("Subcription failure");

    // Wait for mqtt connection to establish and disconnect
    // TODO: BUG -> Remove sleep and the test fails
    thread::sleep(Duration::new(1, 0));
    publisher.disconnect().unwrap();
    // Wait for reconnection and publish
    thread::sleep(Duration::new(10, 0));
    let payload = format!("hello rust");
    publisher.publish("test/reconnect", QoS::Level1, payload.clone().into_bytes()).unwrap();

    // Wait for count to be incremented by callback
    thread::sleep(Duration::new(5, 0));
    assert!(1 == final_count.load(Ordering::SeqCst));
}

#[test]
fn will() {
    let mut client_options = MqttOptions::new();
    let client1 = client_options.set_keep_alive(5)
                                    .set_reconnect(15)
                                    .set_client_id("test-will-c1")
                                    .set_clean_session(false)
                                    .set_will("test/will", "I'm dead")
                                    .connect("broker.hivemq.com:1883");

    let mut client_options = MqttOptions::new();
    let client2 = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .set_client_id("test-will-c2")
                                    .connect("broker.hivemq.com:1883");

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    // Connects to a broker and returns a `Publisher` and `Subscriber`
    // BUG NOTE: don't use _ for dummy subscriber, publisher. That implies
    // channel ends in struct are invalid
    let (mut publisher1, subscriber1) = client1.start().expect("Coudn't start");
    let (publisher2, subscriber2) = client2.
    message_callback(move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        println!("message --> {:?}", message);
    })
    .start().expect("Coudn't start");

    subscriber2.subscribe(vec![("test/will", QoS::Level0)]).unwrap();

    // LWT doesn't work on graceful disconnects
    // publisher1.disconnect();

    // Give some time for mqtt connection to be made on
    // connected socket before shutting down the socket
    // TODO: BUG -> Remove sleep and the test fails
    thread::sleep(Duration::new(1, 0));
    publisher1.shutdown().unwrap();

    // Wait for last will publish
    thread::sleep(Duration::new(5, 0));
    assert!(1 == final_count.load(Ordering::SeqCst));
}