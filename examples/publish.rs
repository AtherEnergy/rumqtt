extern crate rumqtt;
extern crate  pretty_env_logger;
use rumqtt::{MqttOptions, MqttClient};
use std::thread;

fn main() {
    pretty_env_logger::init();
    let mqtt_options = MqttOptions::new("test-id", "localhost:1883");

    let mqtt_client = MqttClient::start(mqtt_options);

    thread::sleep_ms(10000);
    println!("Hello world");
}
