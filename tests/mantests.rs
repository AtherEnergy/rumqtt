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

#[ignore]
#[test]
fn send_ping_reqs_in_time() {
    env_logger::init().unwrap();
    let client_options = MqttOptions::new()
                                    .set_keep_alive(5)
                                    .set_pub_q_len(2)
                                    .set_reconnect(3)
                                    .set_q_timeout(10)
                                    .broker("localhost:1883");

    let request = MqttClient::new(client_options)
                                            .start()
                                            .expect("Coudn't start");

    thread::sleep(Duration::new(60, 0));
}

/// This handles the case where messages in channel are being
/// read but pubacks not being received. 
/// If queue holds 10, recv() stops after queue is full. publish
/// channel has space for 10 more. total request publishes = 20
/// NOTE: broker will remember subscriptions for clean_session=false
/// only for n/w disconnections but not for crashes
#[ignore]
#[test]
fn qos1_pub_block() {
  // env_logger::init().unwrap();
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

    let request = MqttClient::new(client_options).message_callback(move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        println!("message --> {:?}", message);
    }).start().expect("Coudn't start");

    request.subscribe(vec![("test/qos1/block", QoS::Level1)]).expect("Subcription failure");

    println!("Take broker down in next 10 seconds !!!!!!");
    thread::sleep(Duration::new(10, 0));

    for i in 0..10 {
        let payload = format!("{}. hello rust", i);
        println!("{}. Publishing ...", i);
        request.publish("test/qos1/block",  QoS::Level1, payload.clone().into_bytes()).unwrap();
        thread::sleep(Duration::new(0, 10000));
    }

    thread::sleep(Duration::new(20, 0));
    println!("QoS 1 Count = {:?}", final_count.load(Ordering::SeqCst));
    assert!(10 == final_count.load(Ordering::SeqCst));
}

#[ignore]
#[test]
fn tls_connect() {
    env_logger::init().unwrap();
    let client_options = MqttOptions::new()
                                    .set_keep_alive(5)
                                    .set_pub_q_len(10)
                                    .set_reconnect(3)
                                    .set_ca("/usr/local/ca.crt")
                                    .broker("veh-test-mqtt-broker.atherengineering.in:8883");

    //TODO: Alert!!! Mosquitto seems to be unable to publish fast (loosing messsages
    // with mosquitto broker. local and remote)

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let request = MqttClient::new(client_options).message_callback(move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        //println!("message --> {:?}", message);
    }).start().expect("Coudn't start");

    request.subscribe(vec![("test/qos1/stress", QoS::Level1)]).expect("Subcription failure");

    for i in 0..1000 {
        let payload = format!("{}. hello rust", i);
        request.publish("test/qos1/stress",  QoS::Level1, payload.clone().into_bytes()).unwrap();
        thread::sleep(Duration::new(0, 10000));
    }

    thread::sleep(Duration::new(300, 0));
    println!("QoS1 Final Count = {:?}", final_count.load(Ordering::SeqCst));
    assert!(1000 <= final_count.load(Ordering::SeqCst));
}



// #[test]
    // fn retransmission_after_timeout() {
    //     let client_options = MqttOptions::new()
    //         .set_keep_alive(5)
    //         .set_q_timeout(5)
    //         .set_client_id("test-retransmission-client")
    //         .broker(BROKER_ADDRESS);

    //     let mut mq_client = MqttClient::new(client_options);
    //     fill_qos1_publish_buffer(&mut mq_client);
    //     fill_qos2_publish_buffer(&mut mq_client);

    //     let request = mq_client.start().expect("Coudn't start");
    //     thread::sleep(Duration::new(20, 0));
    //     let final_qos1_length = request.qos1_q_len().expect("Stats Request Error");
    //     let final_qos2_length = request.qos2_q_len().expect("Stats Request Error");
    //     assert_eq!(0, final_qos1_length);
    //     assert_eq!(0, final_qos2_length);
    // }