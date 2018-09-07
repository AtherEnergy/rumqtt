extern crate rumqtt;
extern crate  pretty_env_logger;
use rumqtt::{MqttOptions, MqttClient, QoS};
use std::thread;

fn main() {
    pretty_env_logger::init();
    let mqtt_options = MqttOptions::new("test-id-1", "localhost:1883")
                                              .set_keep_alive(10);

    let (mut mqtt_client, notifications) = MqttClient::start(mqtt_options);

    thread::spawn(move || {
        thread::sleep_ms(5000);
        mqtt_client.disconnect().unwrap();
    });

    for notification in notifications {
        println!("{:?}", notification)
    }
}