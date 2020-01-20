
use rumqtt::{MqttClient, MqttOptions, QoS};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Arc;
use std::{thread, time::Duration};
use std::io::{BufRead, BufReader};

#[derive(Serialize, Deserialize)]
struct Publish {
    topic_name: String,
    payload: Arc<Vec<u8>>,
}

fn main() {
    pretty_env_logger::init();
    let mqtt_options = MqttOptions::new("test-id-1", "localhost", 1883).set_keep_alive(10);
    let (mut mqtt_client, notifications) = MqttClient::start(mqtt_options).unwrap();

    thread::spawn(move || {
        for notification in notifications {
            println!("{:?}", notification)
        }

        println!("Exiting notifications");
    });

    mqtt_client.pause().unwrap();
    for i in 1..=10 {
        let payload = format!("publish {}", i);
        thread::sleep(Duration::from_millis(100));
        mqtt_client.publish("hello/world", QoS::AtLeastOnce, false, payload).unwrap();
    }

    let state = mqtt_client.shutdown().unwrap();
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .open("shutdown.bkp")
        .unwrap();

    for publish in state.outgoing_pub {
        println!("Publish = {:?}", publish);
        let p = Publish {
            topic_name: publish.topic_name,
            payload: publish.payload,
        };

        let s = serde_json::to_string(&p).unwrap();
        if let Err(e) = writeln!(file, "{}", s) {
            println!("Write error = {:?}", e);
        }
    }

    let mqtt_options = MqttOptions::new("test-id-1", "localhost", 1883).set_keep_alive(10);
    let (mut mqtt_client, notifications) = MqttClient::start(mqtt_options).unwrap();

    thread::spawn(move || {
        let file = File::open("shutdown.bkp").unwrap();
        for line in BufReader::new(file).lines() {
            let p = line.unwrap();

            let publish: Publish = serde_json::from_str(&p).unwrap();
            mqtt_client.publish("hello/world", QoS::AtLeastOnce, false, (*publish.payload).clone()).unwrap();
        }
    });

    for notification in notifications {
        println!("Notification = {:?}", notification);
    }
}
