use crossbeam_channel::select;
use rumqtt::{MqttClient, MqttOptions, QoS};

use std::{thread, time::Duration};

fn main() {
    pretty_env_logger::init();
    let opts = MqttOptions::builder()
        .client_id("test-id")
        .host("127.0.0.1")
        .port(1883)
        .keep_alive(30)
        .build()
        .unwrap();

    let (mut mqtt_client, notifications) = MqttClient::start(opts).unwrap();
    let (done_tx, done_rx) = crossbeam_channel::bounded(1);

    mqtt_client.subscribe("hello/world", QoS::AtLeastOnce).unwrap();

    let sleep_time = Duration::from_millis(100);

    thread::spawn(move || {
        for i in 0..1000 {
            let payload = format!("publish {}", i);
            thread::sleep(sleep_time);

            mqtt_client.publish("hello/world", QoS::AtLeastOnce, false, payload).unwrap();
        }

        thread::sleep(sleep_time * 10);
        done_tx.send(true).unwrap();
    });

    loop {
        select! {
            recv(notifications) -> notification => {
                println!("{:?}", notification)
            }
            recv(done_rx) -> _done => break
        }
    }
}
