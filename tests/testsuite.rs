extern crate rumqtt;

use rumqtt::{MqttOptions, SslContext, QoS};
use std::thread;
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Shouldn't try to reconnect if there is a connection problem
/// during initial connect.

#[test]
#[should_panic]
fn inital_connect_failure_test() {
    let mut client_options = MqttOptions::new();

    // Specify client connection opthons and which broker to connect to
    let proxy_client = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .connect("localhost:1883");

    // Connects to a broker and returns a `Publisher` and `Subscriber`
    let (_, _) = proxy_client.start().expect("Coudn't start");
}

#[test]
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

    let topics = vec![("test/basic", QoS::Level0)];

    subscriber.subscribe(topics).expect("Subcription failure");  
    subscriber.message_callback(move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        // println!("message --> {:?}", message);
    });
    
    let payload = format!("hello rust");
    publisher.publish("test/basic", QoS::Level0, payload.clone().into_bytes());
    publisher.publish("test/basic", QoS::Level1, payload.clone().into_bytes());
    publisher.publish("test/basic", QoS::Level2, payload.clone().into_bytes());

    thread::sleep(Duration::new(3, 0));
    assert!(3 == final_count.load(Ordering::SeqCst));
    thread::sleep(Duration::new(3, 0));
}

#[test]
fn reconnection_test() {
    // Create client in clean_session = false for broker to
    // remember your subscription after disconnect.
    let mut client_options = MqttOptions::new();
    let proxy_client = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .set_client_id("test-reconnect-client")
                                    .set_clean_session(false)
                                    .connect("test.mosquitto.org:1883");

    let (mut publisher, subscriber) = proxy_client.start().expect("Coudn't start");

    // Message count
    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    // Register message callback and subscribe
    let topics = vec![("test/reconnect", QoS::Level2)];  
    subscriber.message_callback(move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        // println!("message --> {:?}", message);
    });
    subscriber.subscribe(topics).expect("Subcription failure");

    // Wait for mqtt connection to establish and disconnect
    thread::sleep(Duration::new(1, 0));
    publisher.disconnect();

    // Wait for reconnection and publish
    thread::sleep(Duration::new(10, 0));
    let payload = format!("hello rust");
    publisher.publish("test/reconnect", QoS::Level2, payload.clone().into_bytes());

    // Wait for count to be incremented by callback
    thread::sleep(Duration::new(5, 0));
    assert!(1 == final_count.load(Ordering::SeqCst));
}

#[test]
fn will_test() {
    let mut client_options = MqttOptions::new();
    let client1 = client_options.set_keep_alive(5)
                                    .set_reconnect(15)
                                    .set_client_id("test-will-c1")
                                    .set_clean_session(false)
                                    .set_will("test/will", "I'm dead")
                                    .connect("test.mosquitto.org:1883");

    let mut client_options = MqttOptions::new();
    let client2 = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .set_client_id("test-will-c2")
                                    .connect("test.mosquitto.org:1883");

    let (mut publisher1, _) = client1.start().expect("Coudn't start");
    let (_, subscriber2) = client2.start().expect("Coudn't start");

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    subscriber2.message_callback(move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        // println!("message --> {:?}", message);
    });
    subscriber2.subscribe(vec![("test/will", QoS::Level0)]);

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

//------------------------------ MANUAL TESTS -----------------------------------

/// Subscribe and publish at high speed for 1 min
/// and see if PINGREQs are able to keep up
// FIXME: Publish with QoS 2
//#[test]
fn stress_test1() {
    let mut client_options = MqttOptions::new();
    let client = client_options.set_keep_alive(5)
                               .connect("localhost:1883");

    let (publisher, subscriber) = client.start().expect("Coudn't start");
    subscriber.message_callback(|message| println!("{:?}", String::from_utf8((*message.payload).clone())));
    subscriber.subscribe(vec![("hello/rust", QoS::Level2)]);

    for i in 0..100000 {
        let payload = format!("{}. hello rust", i);
        publisher.publish("hello/rust", QoS::Level0, payload.clone().into_bytes());
        thread::sleep(Duration::new(0, 100000)); //1/10th of a second
    }
    thread::sleep(Duration::new(3, 0));
}

/// Publish high frequency data to this topic and see
/// if client is able to send PINGREQs
// for i in `seq 1 1000`;do mosquitto_pub -t "hello/rust" -m "${i} hello rust"; sleep 0.1; done
//#[test]
fn keep_alive_stress_test() {
    let mut client_options = MqttOptions::new();
    let client = client_options.set_keep_alive(5)
                               .connect("localhost:1883");

    let (publisher, subscriber) = client.start().expect("Coudn't start");
    subscriber.message_callback(|message| println!("{:?}", String::from_utf8((*message.payload).clone())));
    subscriber.subscribe(vec![("hello/rust", QoS::Level2)]);
    thread::sleep(Duration::new(60, 0));
}

//#[test]
fn zero_len_client_id_test1() {
    let mut client_options = MqttOptions::new();
    let client = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .set_client_id("")
                                    .set_clean_session(true)
                                    .connect("localhost:1883");

    let _ = client.start().expect("Coudn't start");
}

///#[test]
//#[should_panic]
// This should've failed
fn zero_len_client_id_test2() {
    let mut client_options = MqttOptions::new();
    let client = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .set_client_id("")
                                    .set_clean_session(false)
                                    .connect("localhost:1883");

    let _ = client.start().expect("Coudn't start");
    thread::sleep(Duration::new(5, 0));
}