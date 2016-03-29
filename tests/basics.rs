extern crate mqtt;

use mqtt::client::client::{MqttClient, MqttConnectionOptions};
use mqtt::{TopicFilter, QualityOfService};

use std::thread;
use std::time::Duration;


/// Test publishes along with ping requests and responses
// #[test]
fn publish_test() {
    let mut conn_options = MqttConnectionOptions::new("id2").keep_alive(10);
    let mut client = conn_options.create_client();

    match client.connect("localhost:1883") {
        Ok(result) => println!("Connection successful"),
        Err(_) => panic!("Connectin error"),
    }

    for _ in 0..10 {
        client.publish("hello/world", "hello world", QualityOfService::Level1);
        thread::sleep(Duration::new(1, 0));
    }

    thread::sleep(Duration::new(30, 0));
}


///--> disconnect & reconnect and check if you broker is getting ping requests
/// OBSERVATION: VariablePacket::decode(&mut stream_clone) is an error if socket closes (even if client isn't trying to ping)
///TODO: Analyze weird reconnections after commenting republish & ping req thread
// #[test]
fn reconnection_test() {
    let mut conn_options = MqttConnectionOptions::new("id2").keep_alive(10);
    let mut client = conn_options.create_client();

    match client.connect("localhost:1883") {
        Ok(result) => println!("Connection successful"),
        Err(_) => panic!("Connectin error"),
    }
    thread::sleep(Duration::new(60, 0));
}


///---> Keep publishing packets. disconnect. reconnect. see if failed publishes are being resent
/// IDEA: All the publishes will be added to publish queue (be actual publish successful or not) (till a limit)
///       After reconnection, failed publises won't be getting an ack and they will be republished by republish thread

/// MAYBE: How about user publish just adding publishes to the queue and underlying connection publish
///        doing actual publish and poping the queue only after publish is successful ??
#[test]
fn disconnection_republish_test() {
    let mut conn_options = MqttConnectionOptions::new("id2").keep_alive(5);
    let mut client = conn_options.create_client();

    match client.connect("localhost:1883") {
        Ok(result) => println!("Connection successful"),
        Err(_) => panic!("Connectin error"),
    }

    for _ in 0..10 {
        client.publish("hello/world", "hello world", QualityOfService::Level1);
        thread::sleep(Duration::new(2, 0));
    }

    thread::sleep(Duration::new(120, 0));
}


// #[test]
// fn pingreq_test() {
//     let mut client = MqttClient::new("id2").keep_alive(10);

//     match client.connect("localhost:1883") {
//         Ok(result) => println!("Connection successful"),
//         Err(_) => panic!("Connectin error"),
//     }

//     thread::sleep(Duration::new(30, 0));
// }

// // #[test]
// fn subscribe_test() {
//     let mut client = MqttClient::new("id3").clean_session(true);

//     match client.connect("test.mosquitto.org:1883") {
//         Ok(result) => println!("Subscribe: Connection successful"),
//         Err(_) => panic!("Connectin error"),
//     }

//     let topics: Vec<(TopicFilter, QualityOfService)> =
//         vec![(TopicFilter::new_checked("hello/world".to_string()).unwrap(),
//               QualityOfService::Level0)];

// client.subscribe(topics);

//     for i in 0..10 {
//         let message = format!("{}. Hello Rust Mqtt", i);
//         client.publish("hello/world", &message);
//     }

//     thread::sleep(Duration::new(30, 0));
// }

// #[test]
// fn callback_test() {
//     let mut client = MqttClient::new("id3").clean_session(true).keep_alive(30);
//     client.connect("localhost:1883").unwrap();

//     let topics: Vec<(TopicFilter, QualityOfService)> =
//         vec![(TopicFilter::new_checked("hello/world".to_string()).unwrap(),
//               QualityOfService::Level0)];

// client.subscribe(topics);

//     client.on_message(|a: &str, b: &str| {
//         println!("1. callback...yeahhhh ---> {:?}, {:?}", a, b);
//     });

//     for _ in 0..100 {
//         client.publish("hello/world", "hello world", QualityOfService::Level1);
//     }

// thread::sleep(Duration::new(30, 0));

//     client.on_message(|a: &str, b: &str| {
//         println!("2. callback...yeahhhh ---> {:?}, {:?}", a, b);
//     });

//     thread::sleep(Duration::new(120, 0));
// }
