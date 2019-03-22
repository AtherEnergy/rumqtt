use rumqtt::{MqttClient, MqttOptions, QoS};
use std::{thread, time::Duration};

fn main() {
    pretty_env_logger::init();
    let mut opts = MqttOptions::new("test-id", "127.0.0.1", 1883);
    opts.set_keep_alive(10);

    let (mut mqtt_client, notifications) = MqttClient::start(opts).unwrap();

    mqtt_client.subscribe("hello/world", QoS::AtLeastOnce).unwrap();

    let mut c1 = mqtt_client.clone();
    let mut c2 = mqtt_client.clone();

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
