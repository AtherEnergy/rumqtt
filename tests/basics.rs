extern crate amqtt;
extern crate mqtt;
#[macro_use]
extern crate log;
extern crate env_logger;

use amqtt::ClientOptions;
use mqtt::{TopicFilter, QualityOfService};

use std::thread;
use std::time::Duration;


/// Test publishes along with ping requests and responses
/// Observe if the boker is getting ping requests with in keep_alive time
/// Add handling in client if pingresp isn't received for a ping request
#[test]
fn publish_test() {
    // USAGE: RUST_LOG=amqtt cargo test -- --nocapture
    env_logger::init().unwrap();

    let mut client_options = ClientOptions::new();
    client_options.set_keep_alive(10);

    let mut client = match client_options.connect("localhost:1883") {
        Ok(c) => c,
        Err(_) => panic!("Connectin error"),
    };
    
    client.await();
}


// ---> Keep publishing packets. disconnect. reconnect. see if failed publishes are being resent
// IDEA: All the publishes will be added to publish queue (be actual publish successful or not) (till a limit)
//       After reconnection, failed publises won't be getting an ack and they will be republished by republish thread

// MAYBE: How about user publish just adding publishes to the queue and underlying connection publish
//        doing actual publish and poping the queue only after publish is successful ??

// fn disconnection_republish_test() {
//     let mut conn_options = MqttConnectionOptions::new("id2").keep_alive(5);
//     let mut client = conn_options.create_client();

//     match client.connect("localhost:1883") {
//         Ok(result) => println!("Connection successful"),
//         Err(_) => panic!("Connectin error"),
//     }

//     for _ in 0..10 {
//         client.publish("hello/world", "hello world", QualityOfService::Level1);
//         thread::sleep(Duration::new(2, 0));
//     }

//     thread::sleep(Duration::new(120, 0));
// }
