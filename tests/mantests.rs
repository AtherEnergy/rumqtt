#![allow(unused_variables)]
extern crate rumqtt;

use rumqtt::{MqttOptions, MqttClient, MqttCallback, QoS};
use std::thread;
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
// #[macro_use]
// extern crate log;
// extern crate env_logger;

fn ssl_invalid_ca() {
    // env_logger::init().unwrap();
    let client_options = MqttOptions::new()
        .set_broker("perf-mqtt-broker.atherengineering.in:5000")
        .set_ca("utils/ca_chain_bad.cert.pem")
        .set_client_cert("utils/bike.cert.pem", "utils/bike.key.pem");
    let request = MqttClient::start(client_options, None).expect("Coudn't start");
}

fn ssl_mutual_authentication_pubsub() {
    // env_logger::init().unwrap();
    // TODO: Bugfix. Client hanging when connecting to broker.hivemq.com:9999
    let client_options = MqttOptions::new()
        .set_broker("perf-mqtt-broker.atherengineering.in:5000")
        .set_ca("utils/ca-chain.cert.pem")
        .set_client_cert("utils/s340.cert.pem", "utils/s340.key.pem");

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

// // TEST 1: Check if ping requests are going in time in a stable connection.
// // [working]
// // TEST 2: Disconnect the network while pinging and check if reconnections
// are
// // being tried [working]
// // TEST 3: Reconnect the same network and check if reconnection is successful
// // [working]
// // TEST 4: Reconnect different network and check if reconnection is
// successful
// // [working]
// #[ignore]
// #[test]
// fn ping_reqs_in_time_and_reconnections() {
//     env_logger::init().unwrap();
//     let client_options = MqttOptions::new()
//         .set_keep_alive(10)
//         .set_pub_q_len(2)
//         .set_reconnect(3)
//         .set_q_timeout(10)
//         .set_client_id("test-ping-reqs-in-time")
//         .broker("endurance-broker.atherengineering.in:1883");

//     let request = MqttClient::new(client_options)
//         .start()
//         .expect("Coudn't start");

//     thread::sleep(Duration::new(180, 0));
// }


// // TEST 1: Start and immediately the unplug the cable. Broker will close the
// // socket after timeout. Check if reconnections are being tried
// // [working]
// // TEST 2: Reconnect the same network and check if reconnection is successful
// // and publish
// //         count is proper [working]
// // TEST 3: Reconnect different network and check if reconnection is
// successful
// // and publish
// //         count is proper [working]
// #[ignore]
// #[test]
// fn half_open_publishes_and_reconnections() {
//     env_logger::init().unwrap();
//     let client_options = MqttOptions::new()
//         .set_reconnect(3)
//         .set_client_id("test-ho-publishes")
//         .broker("endurance-broker.atherengineering.in:1883");

//     let count = Arc::new(AtomicUsize::new(0));
//     let final_count = count.clone();
//     let count = count.clone();

//     // Connects to a broker and returns a `request`
//     let request = MqttClient::new(client_options)
//         .publish_callback(move |message| {
//             count.fetch_add(1, Ordering::SeqCst);
//         })
//         .start()
//         .expect("Coudn't start");

//     for i in 0..10_000 {
//         let payload = format!("{}. hello rust", i);
// request.publish("test/half/publish", QoS::Level1,
// payload.clone().into_bytes()).unwrap();
//         thread::sleep(Duration::new(0, 10000));
//     }

//     thread::sleep(Duration::new(60, 0));
//     println!("Final Count = {:?}", final_count.load(Ordering::SeqCst));
//     assert!(10_000 == final_count.load(Ordering::SeqCst));
// }

// /// This handles the case where messages in channel are being
// /// read but pubacks not being received.
// /// If queue holds 10, recv() stops after queue is full. publish
// /// channel has space for 10 more. total request publishes = 20
// /// NOTE: broker will remember subscriptions for clean_session=false
// /// only for n/w disconnections but not for crashes
// #[ignore]
// #[test]
// fn qos1_pub_block() {
//     // env_logger::init().unwrap();
//     let client_options = MqttOptions::new()
//         .set_keep_alive(5)
//         .set_pub_q_len(2)
//         .set_reconnect(3)
//         .set_q_timeout(10)
//         .broker("localhost:1883");

//     // TODO: Alert!!! Mosquitto seems to be unable to publish fast (loosing
//     // messsages
//     // with mosquitto broker. local and remote)
//     let count = Arc::new(AtomicUsize::new(0));
//     let final_count = count.clone();
//     let count = count.clone();

//     let request = MqttClient::new(client_options)
//         .message_callback(move |message| {
//             count.fetch_add(1, Ordering::SeqCst);
//             println!("message --> {:?}", message);
//         })
//         .start()
//         .expect("Coudn't start");

// request.subscribe(vec![("test/qos1/block",
// QoS::Level1)]).expect("Subcription failure");

//     println!("Take broker down in next 10 seconds !!!!!!");
//     thread::sleep(Duration::new(10, 0));

//     for i in 0..10 {
//         let payload = format!("{}. hello rust", i);
//         println!("{}. Publishing ...", i);
// request.publish("test/qos1/block", QoS::Level1,
// payload.clone().into_bytes()).unwrap();
//         thread::sleep(Duration::new(0, 10000));
//     }

//     thread::sleep(Duration::new(20, 0));
//     println!("QoS 1 Count = {:?}", final_count.load(Ordering::SeqCst));
//     assert!(10 == final_count.load(Ordering::SeqCst));
// }

// #[ignore]
// #[test]
// fn tls_connect() {
//     env_logger::init().unwrap();
//     let client_options = MqttOptions::new()
//         .set_keep_alive(5)
//         .set_pub_q_len(10)
//         .set_reconnect(3)
//         .set_ca("/usr/local/ca.crt")
//         .broker("veh-test-mqtt-broker.atherengineering.in:8883");

//     // TODO: Alert!!! Mosquitto seems to be unable to publish fast (loosing
//     // messsages
//     // with mosquitto broker. local and remote)

//     let count = Arc::new(AtomicUsize::new(0));
//     let final_count = count.clone();
//     let count = count.clone();

//     let request = MqttClient::new(client_options)
//         .message_callback(move |message| {
//             count.fetch_add(1, Ordering::SeqCst);
//             // println!("message --> {:?}", message);
//         })
//         .start()
//         .expect("Coudn't start");

// request.subscribe(vec![("test/qos1/stress",
// QoS::Level1)]).expect("Subcription failure");

//     for i in 0..1000 {
//         let payload = format!("{}. hello rust", i);
// request.publish("test/qos1/stress", QoS::Level1,
// payload.clone().into_bytes()).unwrap();
//         thread::sleep(Duration::new(0, 10000));
//     }

//     thread::sleep(Duration::new(300, 0));
//     println!("QoS1 Final Count = {:?}", final_count.load(Ordering::SeqCst));
//     assert!(1000 <= final_count.load(Ordering::SeqCst));
// }



// // #[test]
// // fn retransmission_after_timeout() {
// //     let client_options = MqttOptions::new()
// //         .set_keep_alive(5)
// //         .set_q_timeout(5)
// //         .set_client_id("test-retransmission-client")
// //         .broker(BROKER_ADDRESS);

// //     let mut mq_client = MqttClient::new(client_options);
// //     fill_qos1_publish_buffer(&mut mq_client);
// //     fill_qos2_publish_buffer(&mut mq_client);

// //     let request = mq_client.start().expect("Coudn't start");
// //     thread::sleep(Duration::new(20, 0));
// // let final_qos1_length = request.qos1_q_len().expect("Stats Request
// // Error");
// // let final_qos2_length = request.qos2_q_len().expect("Stats Request
// // Error");
// //     assert_eq!(0, final_qos1_length);
// //     assert_eq!(0, final_qos2_length);
// // }
