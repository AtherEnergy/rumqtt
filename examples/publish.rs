extern crate rumqtt;
extern crate  pretty_env_logger;
use rumqtt::{MqttOptions, MqttClient, QoS};
use std::thread;

fn main() {
    pretty_env_logger::init();
    let mqtt_options = MqttOptions::new("test-id", "localhost:1883");

    let (mut mqtt_client, notifications) = MqttClient::start(mqtt_options);

    thread::spawn(move || {
        for i in 0..100 {
            let payload = format!("publish {}", i);
            thread::sleep_ms(300);
            mqtt_client.publish("hello/world",QoS::AtLeastOnce, payload);
        }
    });

    for notification in notifications {
        println!("{:?}", notification)
    }
}
