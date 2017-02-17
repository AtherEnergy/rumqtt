#![allow(unused_variables)]
extern crate rumqtt;

use rumqtt::{MqttOptions, MqttClient, QoS, MqttCallback, Message};
use std::thread;
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
#[macro_use]
extern crate log;
extern crate env_logger;

const BROKER_ADDRESS: &'static str = "dev-mqtt-broker.atherengineering.in:1883";
const MOSQUITTO_ADDR: &'static str = "test.mosquitto.org:1883";

/// Shouldn't try to reconnect if there is a connection problem
/// during initial tcp connect.
#[test]
#[should_panic]
fn inital_tcp_connect_failure() {
    // env_logger::init().unwrap();
    // TODO: Bugfix. Client hanging when connecting to broker.hivemq.com:9999
    let client_options = MqttOptions::new()
        .set_reconnect(5)
        .set_broker("localhost:9999");

    // Connects to a broker and returns a `request`
    let request = MqttClient::start(client_options, None).expect("Couldn't start");
}

// After connecting to tcp, should timeout error if it didn't receive CONNACK
// before timeout
#[test]
#[should_panic]
fn connect_timeout_failure() {
    // TODO: Change the host to remote host and fix blocks
    let client_options = MqttOptions::new().set_broker("localhost:9999");
    let request = MqttClient::start(client_options, None).expect("Couldn't restart");
}

// Shouldn't try to reconnect if there is a connection problem
// during initial mqtt connect.
#[test]
#[should_panic]
fn inital_mqtt_connect_failure() {
    let client_options = MqttOptions::new()
        .set_reconnect(5)
        .set_broker("test.mosquitto.org:8883");

    // Connects to a broker and returns a `request`
    let client = MqttClient::start(client_options, None).expect("Couldn't start");
}

#[test]
fn basic_publishes_and_subscribes() {
    // env_logger::init().unwrap();
    let client_options = MqttOptions::new()
        .set_reconnect(5)
        .set_broker(MOSQUITTO_ADDR);
    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let counter_cb = move |_| {
        count.fetch_add(1, Ordering::SeqCst);
    };

    let msg_callback = MqttCallback::new().on_message(counter_cb);

    let mut request = MqttClient::start(client_options, Some(msg_callback)).expect("Coudn't start");

    let topics = vec![("test/basic", QoS::Level0)];
    request.subscribe(topics).expect("Subcription failure");
    let payload = format!("hello rust");
    request.publish("test/basic", QoS::Level0, payload.clone().into_bytes())
        .unwrap();
    request.publish("test/basic", QoS::Level1, payload.clone().into_bytes())
        .unwrap();
    request.publish("test/basic", QoS::Level2, payload.clone().into_bytes())
        .unwrap();
    thread::sleep(Duration::new(3, 0));

    assert_eq!(3, final_count.load(Ordering::SeqCst));
}

#[test]
fn simple_reconnection() {
    // env_logger::init().unwrap();
    let client_options = MqttOptions::new()
        .set_keep_alive(5)
        .set_reconnect(5)
        .set_client_id("test-reconnect-client")
        .set_broker(MOSQUITTO_ADDR);

    // Message count
    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let counter_cb = move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        // println!("message --> {:?}", message);
    };

    let msg_callback = MqttCallback::new().on_message(counter_cb);

    // Connects to a broker and returns a `request`
    let mut request = MqttClient::start(client_options, Some(msg_callback)).expect("Coudn't start");

    // Register message callback and subscribe
    let topics = vec![("test/reconnect", QoS::Level2)];
    request.subscribe(topics).expect("Subcription failure");

    request.disconnect().unwrap();
    // Wait for reconnection and publish
    thread::sleep(Duration::new(10, 0));

    let payload = format!("hello rust");
    request.publish("test/reconnect", QoS::Level1, payload.clone().into_bytes())
        .unwrap();

    // Wait for count to be incremented by callback
    thread::sleep(Duration::new(5, 0));
    assert!(1 == final_count.load(Ordering::SeqCst));
}

#[test]
fn acked_message() {
    let client_options = MqttOptions::new()
        .set_reconnect(5)
        .set_client_id("test-reconnect-client")
        .set_broker(MOSQUITTO_ADDR);

    let cb = |m: Message| {
        let ref payload = *m.payload;
        let ref userdata = *m.userdata.unwrap();
        let payload = String::from_utf8(payload.clone()).unwrap();
        let userdata = String::from_utf8(userdata.clone()).unwrap();
        assert_eq!("MYUNIQUEMESSAGE".to_string(), payload);
        assert_eq!("MYUNIQUEUSERDATA".to_string(), userdata);
    };

    let msg_callback = MqttCallback::new().on_message(cb);

    // Connects to a broker and returns a `request`
    let mut request = MqttClient::start(client_options, Some(msg_callback)).expect("Couldn't start");
    request.userdata_publish("test/qos1/ack",
                          QoS::Level1,
                          "MYUNIQUEMESSAGE".to_string().into_bytes(),
                          "MYUNIQUEUSERDATA".to_string().into_bytes())
        .unwrap();
    thread::sleep(Duration::new(1, 0));
}

#[test]
fn will() {
    // env_logger::init().unwrap();
    let client_options1 = MqttOptions::new()
        .set_reconnect(15)
        .set_client_id("test-will-c1")
        .set_clean_session(false)
        .set_will("test/will", "I'm dead")
        .set_broker(MOSQUITTO_ADDR);

    let client_options2 = MqttOptions::new()
        .set_keep_alive(5)
        .set_reconnect(5)
        .set_client_id("test-will-c2")
        .set_broker(MOSQUITTO_ADDR);

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    // BUG NOTE: don't use _ for dummy subscriber, request. That implies
    // channel ends in struct are invalid

    let cb = move |message| {
        count.fetch_add(1, Ordering::SeqCst);
        println!("message --> {:?}", message);
    };

    let callback = MqttCallback::new().on_message(cb);

    // let client1 = MqttClient::start(client_options1, None).expect("Coudn't
    // start");
    // let mut client2 = MqttClient::start(client_options2,
    // Some(callback)).expect("Coudn't start");

    // client2.subscribe(vec![("test/will", QoS::Level0)]).unwrap();

    // TODO: Now we are waiting for cli-2 subscriber to finish before
    // disconnecting
    // cli-1. Make an sync version of subscribe()

    thread::sleep(Duration::new(1, 0));

    // LWT doesn't work on graceful disconnects
    // client1.disconnect();
    // client1.shutdown().unwrap();

    // Wait for last will publish
    // thread::sleep(Duration::new(5, 0));
    // assert!(1 == final_count.load(Ordering::SeqCst));
}

/// Broker should retain published message on a topic and
/// INSTANTLY publish them to new subscritions
#[test]
fn retained_messages() {
    // env_logger::init().unwrap();
    let client_options = MqttOptions::new()
        .set_reconnect(3)
        .set_client_id("test-retain-client")
        .set_clean_session(true)
        .set_broker(BROKER_ADDRESS);
    // NOTE: QoS 2 messages aren't being retained in "test.mosquitto.org"
    // broker

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let cb = move |m: Message| {
        count.fetch_add(1, Ordering::SeqCst);
    };

    let callback = MqttCallback::new().on_message(cb);

    let mut client = MqttClient::start(client_options, Some(callback)).expect("Coudn't start");

    // publish first
    let payload = format!("hello rust");
    client.retained_publish("test/0/retain", QoS::Level0, payload.clone().into_bytes())
        .unwrap();
    client.retained_publish("test/1/retain", QoS::Level1, payload.clone().into_bytes())
        .unwrap();
    client.retained_publish("test/2/retain", QoS::Level2, payload.clone().into_bytes())
        .unwrap();

    // NOTE: Request notifications are on different mio channels. We don't
    // know
    // about priority. Wait till all the publishes are recived by connection
    // thread
    // before disconnection
    thread::sleep(Duration::new(1, 0));
    client.disconnect().unwrap();

    // wait for client to reconnect
    thread::sleep(Duration::new(10, 0));

    // subscribe to the topic which broker has retained
    let topics = vec![("test/+/retain", QoS::Level0)];
    client.subscribe(topics).expect("Subcription failure");

    // wait for messages
    thread::sleep(Duration::new(3, 0));
    assert!(3 == final_count.load(Ordering::SeqCst));
    // TODO: Clear retained messages
}

// TODO: Add functionality to handle noreconnect option. This test case is
// panicking
// with out set_reconnect
#[test]
fn qos0_stress_publish() {
    let client_options = MqttOptions::new()
        .set_reconnect(3)
        .set_client_id("qos0-stress-publish")
        .set_broker(BROKER_ADDRESS);

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let cb = move |m: Message| {
        count.fetch_add(1, Ordering::SeqCst);
    };

    let callback = MqttCallback::new().on_message(cb);

    let mut client = MqttClient::start(client_options, Some(callback)).expect("Coudn't start");

    client.subscribe(vec![("test/qos0/stress", QoS::Level2)]).expect("Subcription failure");

    for i in 0..1000 {
        let payload = format!("{}. hello rust", i);
        client.publish("test/qos0/stress", QoS::Level0, payload.clone().into_bytes()).unwrap();
        thread::sleep(Duration::new(0, 10000));
    }

    thread::sleep(Duration::new(10, 0));
    println!("QoS0 Final Count = {:?}", final_count.load(Ordering::SeqCst));
    assert!(950 <= final_count.load(Ordering::SeqCst));
}

#[test]
fn simple_qos1_stress_publish() {
    // env_logger::init().unwrap();
    let client_options = MqttOptions::new()
        .set_reconnect(3)
        .set_client_id("qos1-stress-publish")
        .set_pub_q_len(50)
        .set_broker(BROKER_ADDRESS);

    // TODO: Alert!!! Mosquitto seems to be unable to publish fast (loosing
    // messsages
    // with mosquitto broker. local and remote)

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let cb = move |m: Message| {
        count.fetch_add(1, Ordering::SeqCst);
    };

    let callback = MqttCallback::new().on_publish(cb);

    let mut client = MqttClient::start(client_options, Some(callback)).expect("Coudn't start");

    for i in 0..1000 {
        let payload = format!("{}. hello rust", i);
        client.publish("test/qos1/stress", QoS::Level1, payload.clone().into_bytes()).unwrap();
    }

    thread::sleep(Duration::new(10, 0));
    println!("QoS1 Final Count = {:?}", final_count.load(Ordering::SeqCst));
    assert!(1000 <= final_count.load(Ordering::SeqCst));
}

#[test]
/// This test tests if all packets are being published after reconnections
/// NOTE: Previous tests used subscribes to same topic to decide if all the
/// publishes are successful. When reconnections are involved, some publishes
/// might happen before subscription is successful. You can verify this by
/// keeping prints at CONNACK, SUBACK & _PUBLISH(). After connection is
/// successful
/// you'll see some publishes before SUBACK.
fn qos1_stress_publish_with_reconnections() {
    // env_logger::init().unwrap();
    let client_options = MqttOptions::new()
        .set_reconnect(3)
        .set_client_id("qos1-stress-reconnect-publish")
        .set_clean_session(false)
        .set_pub_q_len(50)
        .set_broker(BROKER_ADDRESS);

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let cb = move |m: Message| {
        count.fetch_add(1, Ordering::SeqCst);
    };

    let callback = MqttCallback::new().on_publish(cb);

    let mut client = MqttClient::start(client_options, Some(callback)).expect("Coudn't start");

    for i in 0..1000 {
        let payload = format!("{}. hello rust", i);
        if i == 100 || i == 500 || i == 900 {
            let _ = client.disconnect();
        }
        client.publish("test/qos1/reconnection_stress", QoS::Level1, payload.clone().into_bytes()).unwrap();
    }

    thread::sleep(Duration::new(30, 0));
    println!("QoS1 Final Count = {:?}", final_count.load(Ordering::SeqCst));
    assert!(1000 <= final_count.load(Ordering::SeqCst));
}

#[test]
fn simple_qos2_stress_publish() {
    // env_logger::init().unwrap();
    let client_options = MqttOptions::new()
        .set_reconnect(3)
        .set_client_id("qos2-stress-publish")
        .set_broker(BROKER_ADDRESS);

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let cb = move |m: Message| {
        count.fetch_add(1, Ordering::SeqCst);
    };

    let callback = MqttCallback::new().on_publish(cb);

    let mut client = MqttClient::start(client_options, Some(callback)).expect("Coudn't start");


    for i in 0..1000 {
        let payload = format!("{}. hello rust", i);
        client.publish("test/qos2/stress", QoS::Level2, payload.clone().into_bytes()).unwrap();
    }

    thread::sleep(Duration::new(30, 0));
    println!("QoS2 Final Count = {:?}", final_count.load(Ordering::SeqCst));
    assert!(1000 == final_count.load(Ordering::SeqCst));
}

#[test]
fn qos2_stress_publish_with_reconnections() {
    // env_logger::init().unwrap();
    let client_options = MqttOptions::new()
        .set_reconnect(3)
        .set_clean_session(false)
        .set_client_id("qos2-stress-reconnect-publish")
        .set_broker(BROKER_ADDRESS);

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let cb = move |m: Message| {
        count.fetch_add(1, Ordering::SeqCst);
    };

    let callback = MqttCallback::new().on_publish(cb);

    let mut client = MqttClient::start(client_options, Some(callback)).expect("Coudn't start");


    for i in 0..1000 {
        let payload = format!("{}. hello rust", i);
        if i == 40 || i == 500 || i == 900 {
            let _ = client.disconnect();
        }
        client.publish("test/qos2/reconnection_stress", QoS::Level2, payload.clone().into_bytes()).unwrap();
    }

    thread::sleep(Duration::new(30, 0));
    println!("QoS2 Final Count = {:?}", final_count.load(Ordering::SeqCst));
    assert!(1000 == final_count.load(Ordering::SeqCst));
}


// NOTE: POTENTIAL MOSQUITTO BUG
// client publishing 1..40 and disconnect and 40..46(with errors) before read
// triggered
// broker receives 1..40 but sends acks for only 1..20  (don't know why)
// client reconnects and sends 21..46 again and received pubrecs (qos2 publish
// queue empty)
// broker now sends pubrecs from 21..X resulting in unsolicited records

// doesn't seem to be a problem with qos1
// emqttd doesn't have this issue

// #[test]
// fn qos2_stress_publish_with_reconnections() {
//     env_logger::init().unwrap();
//     let client_options = MqttOptions::new()
//                                     .set_keep_alive(5)
//                                     .set_reconnect(3)
//                                     .set_clean_session(false)
//
// .set_client_id("qos2-stress-reconnect-publish")
//                                     .set_broker(BROKER_ADDRESS);

//     let count = Arc::new(AtomicUsize::new(0));
//     let final_count = count.clone();
//     let count = count.clone();

// let request = MqttClient::new(client_options).publish_callback(move
// |message| {
//         count.fetch_add(1, Ordering::SeqCst);
// // println!("{}. message --> {:?}", count.load(Ordering::SeqCst),
// message);
//     }).start().expect("Coudn't start");

//     for i in 0..50 {
//         let payload = format!("{}. hello rust", i);
//         if i == 40 || i == 500 || i == 900 {
//             let _ = request.disconnect();
//         }
// request.publish("test/qos2/reconnection_stress",  QoS::Level2,
// payload.clone().into_bytes()).unwrap();
//     }

//     thread::sleep(Duration::new(10, 0));
//     println!("QoS2 Final Count = {:?}", final_count.load(Ordering::SeqCst));
//     assert!(50 == final_count.load(Ordering::SeqCst));
// }
