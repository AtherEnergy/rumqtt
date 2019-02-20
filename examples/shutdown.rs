use rumqtt::{MqttClient, MqttOptions, QoS};
use std::{thread, time::Duration};

fn main() {
    pretty_env_logger::init();
    let mqtt_options = MqttOptions::new("test-id-1", "localhost", 1883).set_keep_alive(10);

    let (mut mqtt_client, notifications) = MqttClient::start(mqtt_options).unwrap();

    thread::spawn(move || {
        thread::sleep(Duration::from_secs(5));

        for i in 1..11 {
            let payload = format!("publish {}", i);
            thread::sleep(Duration::from_millis(100));
            mqtt_client.publish("hello/world", QoS::AtLeastOnce, false, payload).unwrap();
        }

        mqtt_client.shutdown().unwrap();

        for i in 11..21 {
            let payload = format!("publish {}", i);
            thread::sleep(Duration::from_millis(100));
            mqtt_client.publish("hello/world", QoS::AtLeastOnce, false, payload).unwrap();
        }
    });

    for notification in notifications {
        println!("{:?}", notification)
    }
}
