use rumqtt::{MqttClient, MqttOptions, QoS};
use std::{thread, time::Duration};

fn main() {
    pretty_env_logger::init();
    let mqtt_options = MqttOptions::new("test-id", "127.0.0.1", 1883).set_keep_alive(10);

    let (mqtt_client, notifications) = MqttClient::start(mqtt_options).unwrap();

    mqtt_client.subscribe("hello/world", QoS::AtLeastOnce).unwrap();

    let c1 = mqtt_client.clone();
    let c2 = mqtt_client.clone();

    thread::spawn(move || {
        let dur = Duration::new(1, 0);

        for i in 0..100 {
            let payload = format!("publish {}", i);
            thread::sleep(dur);
            c1.publish("hello/world", QoS::AtLeastOnce, false, payload).unwrap();
        }
    });

    thread::spawn(move || {
        let dur = Duration::new(5, 0);

        for i in 0..100 {
            if i % 2 == 0 {
                c2.pause().unwrap();
            } else {
                c2.resume().unwrap();
            }

            thread::sleep(dur);
        }
    });

    for notification in notifications {
        println!("{:?}", notification)
    }
}
