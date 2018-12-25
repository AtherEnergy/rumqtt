use rumqtt::{MqttClient, MqttOptions, QoS};
use std::{thread, time::Duration};

fn main() {
    pretty_env_logger::init();
    let broker = "prod-mqtt-broker.atherengineering.in";
    let port = 1883;
    let mqtt_options = MqttOptions::new("test-pubsub1", broker, port).set_keep_alive(10);

    let (mut mqtt_client, notifications) = MqttClient::start(mqtt_options).unwrap();

    mqtt_client.subscribe("hello/world", QoS::AtLeastOnce).unwrap();

    thread::spawn(move || {
        for i in 0..100 {
            let payload = format!("publish {}", i);
            thread::sleep(Duration::from_millis(100));
            mqtt_client.publish("hello/world", QoS::AtLeastOnce, payload).unwrap();
        }
    });

    for notification in notifications {
        println!("{:?}", notification)
    }
}
