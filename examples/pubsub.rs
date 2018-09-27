extern crate rumqtt;
extern crate  pretty_env_logger;
use rumqtt::{MqttOptions, MqttClient, QoS};
use std::thread;

fn main() {
    pretty_env_logger::init();
    let mqtt_options = MqttOptions::new("test-id", "127.0.0.1:1883")
        .set_keep_alive(10);

    let (mut mqtt_client, notifications) = MqttClient::start(mqtt_options);

    mqtt_client.subscribe("hello/world", QoS::AtMostOnce).unwrap();

    thread::spawn(move || {
        for i in 0..100 {
            let payload = format!("publish {}", i);
            thread::sleep_ms(1000);
            mqtt_client.publish("hello/world",QoS::AtMostOnce, payload).unwrap();
        }
    });

    for notification in notifications {
        println!("{:?}", notification)
    }
}
