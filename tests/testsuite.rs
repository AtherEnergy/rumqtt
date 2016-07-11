extern crate rumqtt;

use rumqtt::{MqttOptions, SslContext, QoS};
use std::thread;
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};


//#[test]
fn basic_test() {
    let mut client_options = MqttOptions::new();

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

    subscriber.subscribe(topics).expect("Subcription failure");  
    subscriber.message_callback(move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        println!("message --> {:?}", message);
    });
    
    let payload = format!("hello rust");
    publisher.publish("hello/rust", QoS::Level0, payload.clone().into_bytes());
    publisher.publish("hello/rust", QoS::Level1, payload.clone().into_bytes());
    publisher.publish("hello/rust", QoS::Level2, payload.clone().into_bytes());

    thread::sleep(Duration::new(3, 0));
    assert!(3 == final_count.load(Ordering::SeqCst));
    thread::sleep(Duration::new(3, 0));
}

//#[test]
fn retained_message_test() {
    let mut client_options = MqttOptions::new();
    let proxy_client = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .set_client_id("client-1")
                                    .set_clean_session(true)
                                    .connect("localhost:1883");

    let (mut publisher, subscriber) = proxy_client.start().expect("Coudn't start");

    let payload = format!("hello rust");
    publisher.set_retain(true).publish("hello/1/world", QoS::Level0, payload.clone().into_bytes());
    publisher.set_retain(true).publish("hello/2/world", QoS::Level1, payload.clone().into_bytes());
    publisher.set_retain(true).publish("hello/3/world", QoS::Level2, payload.clone().into_bytes());

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let topics = vec![("hello/world", QoS::Level0)];
    subscriber.subscribe(topics).expect("Subcription failure");
    subscriber.message_callback(move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        println!("message --> {:?}", message);
    });

    publisher.disconnect();

    thread::sleep(Duration::new(180, 0));
    assert!(3 == final_count.load(Ordering::SeqCst));
    thread::sleep(Duration::new(3, 0));
}

#[test]
fn will_test() {
    let mut client_options = MqttOptions::new();
    let client1 = client_options.set_keep_alive(5)
                                    .set_reconnect(15)
                                    .set_client_id("client-1")
                                    .set_clean_session(false)
                                    .set_will("hello/world", "I'm dead")
                                    .connect("localhost:1883");

    let mut client_options = MqttOptions::new();
    let client2 = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .set_client_id("client-2")
                                    .connect("localhost:1883");

    let (mut publisher1, _) = client1.start().expect("Coudn't start");
    let (_, subscriber2) = client2.start().expect("Coudn't start");

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    subscriber2.message_callback(move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        println!("message --> {:?}", message);
    });
    subscriber2.subscribe(vec![("hello/world", QoS::Level0)]);

    // LWT doesn't work on graceful disconnects
    // publisher1.disconnect();

    // Give some time for mqtt connection to be made on
    // connected socket before shutting down the socket
    thread::sleep(Duration::new(1, 0));
    publisher1.shutdown();

    // Wait for last will publish
    thread::sleep(Duration::new(5, 0));
    assert!(1 == final_count.load(Ordering::SeqCst));
}