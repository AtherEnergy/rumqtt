extern crate rumqtt;
extern crate  pretty_env_logger;
use rumqtt::{MqttOptions, MqttClient};
use std::thread;

fn main() {
    pretty_env_logger::init();
    let mqtt_options = MqttOptions::new("test-id", "localhost:1883");

    let (mqtt_client, notifications) = MqttClient::start(mqtt_options);

    for notification in notifications {
        println!("{:?}", notification)
    }
}
