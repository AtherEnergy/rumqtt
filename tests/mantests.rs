#![allow(unused_variables)] 
extern crate rumqtt;

use rumqtt::{MqttOptions, MqttClient, QoS};
use std::thread;
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
#[macro_use]
extern crate log;
extern crate env_logger;

/// This handles the case where messages in channel are being
/// read but pubacks not being received. 
/// If queue holds 10, recv() stops after queue is full. publish
/// channel has space for 10 more. total publisher publishes = 20
/// NOTE: broker will remember subscriptions for clean_session=false
/// only for n/w disconnections but not for crashes
//#[ignore]
#[test]
fn qos1_pub_block() {
  env_logger::init().unwrap();
  let client_options = MqttOptions::new()
                                    .set_keep_alive(5)
                                    .set_pub_q_len(2)
                                    .set_reconnect(3)
                                    .set_q_timeout(10)
                                    .broker("localhost:1883");

    //TODO: Alert!!! Mosquitto seems to be unable to publish fast (loosing messsages
    // with mosquitto broker. local and remote)
    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let (publisher, subscriber) = MqttClient::new(client_options).message_callback(move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        // println!("message --> {:?}", message);
    }).start().expect("Coudn't start");

    subscriber.subscribe(vec![("test/qos1/block", QoS::Level1)]).expect("Subcription failure");

    println!("Take broker down in next 10 seconds !!!!!!");
    thread::sleep(Duration::new(10, 0));

    for i in 0..10 {
        let payload = format!("{}. hello rust", i);
        println!("{}. Publishing ...", i);
        publisher.publish("test/qos1/block", QoS::Level1, payload.clone().into_bytes()).unwrap();
        thread::sleep(Duration::new(0, 10000));
    }

    thread::sleep(Duration::new(10, 0));
    println!("QoS 1 Count = {:?}", final_count.load(Ordering::SeqCst));
    assert!(10 == final_count.load(Ordering::SeqCst));
}

#[ignore]
#[test]
fn tls_connect() {
    let client_options = MqttOptions::new()
                                    .set_keep_alive(5)
                                    .set_pub_q_len(10)
                                    .set_reconnect(3)
                                    .set_tls("/Users/ravitejareddy/Dropbox/mosquitto_certs/ca.crt")
                                    .broker("localhost:8883");

    //TODO: Alert!!! Mosquitto seems to be unable to publish fast (loosing messsages
    // with mosquitto broker. local and remote)

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let (publisher, subscriber) = MqttClient::new(client_options).message_callback(move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        println!("message --> {:?}", message);
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