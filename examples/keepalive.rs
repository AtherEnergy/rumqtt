extern crate pretty_env_logger;
extern crate rumqtt;
use rumqtt::{MqttClient, MqttOptions};

fn main() {
    pretty_env_logger::init();
    let mqtt_options = MqttOptions::new("test-id", "127.0.0.1", 1883).set_keep_alive(10);

    let (mut _mqtt_client, notifications) = MqttClient::start(mqtt_options).unwrap();

    for notification in notifications {
        println!("{:?}", notification)
    }
}
