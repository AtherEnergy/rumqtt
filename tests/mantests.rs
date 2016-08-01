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
/// read but pubacks not being received. Fishy. Analyze blocks
/// in regards to channel and queue.
/// If queue holds 10 and then recv() stops, shouldn't 20 publishes
/// happen from publisher ??
//#[ignore]
#[test]
fn qos1_pub_block() {
  env_logger::init().unwrap();
  let client_options = MqttOptions::new()
                                    .set_keep_alive(5)
                                    .set_pub_q_len(10)
                                    .set_reconnect(3)
                                    .set_q_timeout(10)
                                    .set_client_id("test-pub-block")
                                    .set_clean_session(false)
                                    .broker("localhost:1883");

    //TODO: Alert!!! Mosquitto seems to be unable to publish fast (loosing messsages
    // with mosquitto broker. local and remote)
    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let (publisher, subscriber) = MqttClient::new(client_options).message_callback(move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        println!("message --> {:?}", message);
    }).start().expect("Coudn't start");

    subscriber.subscribe(vec![("test/qos1/block", QoS::Level1)]).expect("Subcription failure");

    println!("Take broker down in next 10 seconds !!!!!!");
    thread::sleep(Duration::new(10, 0));

    for i in 0..100 {
        let payload = format!("{}. hello rust", i);
        println!("{}. Publishing ...", i);
        publisher.publish("test/qos1/block", QoS::Level1, payload.clone().into_bytes()).unwrap();
        thread::sleep(Duration::new(0, 10000));
    }

    thread::sleep(Duration::new(20, 0));
    println!("{:?}", final_count.load(Ordering::SeqCst));
    assert!(100 == final_count.load(Ordering::SeqCst));
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