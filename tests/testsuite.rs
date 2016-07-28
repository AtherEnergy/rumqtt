#![allow(unused_variables)] 
extern crate rumqtt;

use rumqtt::{MqttOptions, QoS};
use std::thread;
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
#[macro_use]
extern crate log;
extern crate env_logger;


/// Shouldn't try to reconnect if there is a connection problem
/// during initial tcp connect.
#[test]
#[should_panic]
fn inital_tcp_connect_failure(){
    //env_logger::init().unwrap();
    let mut client_options = MqttOptions::new();

    // Specify client connection opthons and which broker to connect to
    // TODO: Bugfix. Client hanging when connecting to broker.hivemq.com:9999
    // TODO: Rename proxy_client
    let proxy_client = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .connect("localhost:9999");

    // Connects to a broker and returns a `Publisher` and `Subscriber`
    let (_, _) = proxy_client.start().expect("Couldn't start");
}

/// Shouldn't try to reconnect if there is a connection problem
/// during initial mqtt connect.
#[test]
#[should_panic]
fn inital_mqtt_connect_failure() {
    let mut client_options = MqttOptions::new();

    // Specify client connection opthons and which broker to connect to
    let proxy_client = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .connect("test.mosquitto.org:8883");

    // Connects to a broker and returns a `Publisher` and `Subscriber`
    let (_, _) = proxy_client.start().expect("Couldn't start");
}

#[test]
fn basic() {
    //env_logger::init().unwrap();
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
        //println!("message --> {:?}", message);
    }).start().expect("Couldn't start");

    // TODO: Because of async io We are publishing before connection is actually made
    // synchronize this and remove sleep
    //thread::sleep(Duration::new(1, 0));
    let topics = vec![("test/basic", QoS::Level0)];

    subscriber.subscribe(topics).expect("Subcription failure");  
    
    let payload = format!("hello rust");
    publisher.publish("test/basic", QoS::Level0, payload.clone().into_bytes()).unwrap();
    publisher.publish("test/basic", QoS::Level1, payload.clone().into_bytes()).unwrap();
    publisher.publish("test/basic", QoS::Level2, payload.clone().into_bytes()).unwrap();

    thread::sleep(Duration::new(1, 0));
    assert!(3 == final_count.load(Ordering::SeqCst));
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
    // thread::sleep(Duration::new(1, 0));
    publisher.disconnect().unwrap();
    // Wait for reconnection and publish
    thread::sleep(Duration::new(10, 0));
    let payload = format!("hello rust");
    //TODO: This is failing if client is not able to reconnect.
    //Ideally, this shouldn't fail but block
    //Add a testcase where broker/internet is down for some time and this should block
    //instead of failing
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
    let (publisher1, subscriber1) = client1.start().expect("Coudn't start");
    let (publisher2, subscriber2) = client2.
    message_callback(move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        //println!("message --> {:?}", message);
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

/// Broker should retain published message on a topic and
/// INSTANTLY publish them to new subscritions
#[test]
fn retained_messages() {
    //env_logger::init().unwrap();
    let mut client_options = MqttOptions::new();
    let proxy_client = client_options.set_keep_alive(5)
                                    .set_reconnect(3)
                                    .set_client_id("test-retain-client")
                                    .set_clean_session(true)
                                    .connect("broker.hivemq.com:1883");
    //NOTE: QoS 2 messages aren't being retained in "test.mosquitto.org" broker

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let (mut publisher, subscriber) = proxy_client.
    message_callback(move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        //println!("message --> {:?}", message);
    }).start().expect("Coudn't start");

    // publish first
    let payload = format!("hello rust");
    publisher.set_retain(true).publish("test/0/retain", QoS::Level0, payload.clone().into_bytes()).unwrap();
    publisher.set_retain(true).publish("test/1/retain", QoS::Level1, payload.clone().into_bytes()).unwrap();
    publisher.set_retain(true).publish("test/2/retain", QoS::Level2, payload.clone().into_bytes()).unwrap();

    //TODO: BUG -> Wait for some time before sending
    //disconnect packet or else EOF isn't being detected
    //in eventloop and reconnect isn't working
    //NOTE: sleep in client.rs before disconnect isn't working
    //. Strange..
    //thread::sleep(Duration::new(1, 0));
    publisher.disconnect().unwrap();

    // wait for client to reconnect
    thread::sleep(Duration::new(10, 0));

    // subscribe to the topic which broker has retained
    let topics = vec![("test/+/retain", QoS::Level0)];
    subscriber.subscribe(topics).expect("Subcription failure");

    // wait for messages
    thread::sleep(Duration::new(3, 0));
    assert!(3 == final_count.load(Ordering::SeqCst));
    //TODO: Clear retained messages
}

#[test]
fn qos0_stress_publish() {
    let mut client_options = MqttOptions::new();
    let client = client_options.set_keep_alive(5)
                               .set_client_id("qos0-stress-publish")
                               .connect("broker.hivemq.com:1883");

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let (publisher, subscriber) = client.message_callback(move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        // println!("message --> {:?}", message);
    }).start().expect("Coudn't start");

    subscriber.subscribe(vec![("test/qos0/stress", QoS::Level2)]).expect("Subcription failure");

    for i in 0..1000 {
        let payload = format!("{}. hello rust", i);
        publisher.publish("test/qos0/stress", QoS::Level0, payload.clone().into_bytes()).unwrap();
        thread::sleep(Duration::new(0, 10000));
    }

    thread::sleep(Duration::new(10, 0));
    println!("QoS0 Final Count = {:?}", final_count.load(Ordering::SeqCst));
    assert!(1000 == final_count.load(Ordering::SeqCst));
}


#[test]
fn qos1_stress_publish() {
    //env_logger::init().unwrap();
    let mut client_options = MqttOptions::new();
    let client = client_options.set_keep_alive(5)
                               .set_client_id("qos1-stress-publish")
                               .set_pub_q_len(50)
                               .connect("broker.hivemq.com:1883");
    //TODO: Alert!!! Mosquitto seems to be unable to publish fast (loosing messsages
    // with mosquitto broker. local and remote)
    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let (publisher, subscriber) = client.message_callback(move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        // println!("message --> {:?}", message);
    }).start().expect("Coudn't start");

    subscriber.subscribe(vec![("test/qos1/stress", QoS::Level1)]).expect("Subcription failure");

    for i in 0..1000 {
        let payload = format!("{}. hello rust", i);
        publisher.publish("test/qos1/stress", QoS::Level1, payload.clone().into_bytes()).unwrap();
        thread::sleep(Duration::new(0, 10000));
    }

    thread::sleep(Duration::new(300, 0));
    println!("QoS1 Final Count = {:?}", final_count.load(Ordering::SeqCst));
    assert!(1000 <= final_count.load(Ordering::SeqCst));
}

#[test]
fn qos2_stress_publish() {
    let mut client_options = MqttOptions::new();
    let client = client_options.set_keep_alive(5)
                               .set_client_id("qos2-stress-publish")
                               .connect("broker.hivemq.com:1883");

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let (publisher, subscriber) = client.message_callback(move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        //println!("message --> {:?}", message);
    }).start().expect("Coudn't start");
    
    subscriber.subscribe(vec![("test/qos2/stress", QoS::Level2)]).expect("Subcription failure");

    for i in 0..1000 {
        let payload = format!("{}. hello rust", i);
        publisher.publish("test/qos2/stress", QoS::Level2, payload.clone().into_bytes()).unwrap();
        thread::sleep(Duration::new(0, 10000));
    }

    thread::sleep(Duration::new(1000, 0));
    println!("QoS2 Final Count = {:?}", final_count.load(Ordering::SeqCst));
    assert!(1000 == final_count.load(Ordering::SeqCst));
}

//TODO 1: Publish 100000 Qos0, 1, 2 messages and check received count (subscribing to same topic)
//TODO 2: Perform 1 with big messages
//TODO 3: Perform 1 with internet constantly going down
//TODO 4: Perform 2 + 3
//TODO 5: Multiple clients connecting with same client id