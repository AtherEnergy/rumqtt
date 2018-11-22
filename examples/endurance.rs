#[macro_use]
extern crate crossbeam_channel;
extern crate pretty_env_logger;
extern crate rumqtt;

use rumqtt::client::Notification;
use rumqtt::Receiver;
use rumqtt::{MqttClient, MqttOptions, QoS};
use std::{thread, time::Duration};
use std::collections::HashSet;

const COUNT: u16 = 10_000;


fn handle_notifications(notifications: Receiver<Notification>, done: Receiver<bool>) {
    let mut counts = HashSet::new();
    for i in 0..COUNT {
        counts.insert(i + 1);
    }

    loop {
        select! {
            recv(notifications) -> notification => {
                let notification = notification.unwrap();
                println!("{:?}", notification);

                match notification {
                   Notification::Publish(publish) => {
                       let pkid: u16 = publish.pkid.unwrap().into();
                       counts.remove(&pkid);
                   },
                   _ => (),
                }
            }
            recv(done) -> _done => break
        }
    }

    println!("{:?}", counts);
}



fn main() {
    pretty_env_logger::init();
    let mqtt_options = MqttOptions::new("test-id", "127.0.0.1", 1883).set_keep_alive(30);
    let (mut mqtt_client, notifications) = MqttClient::start(mqtt_options).unwrap();
    let (done_tx, done_rx) = crossbeam_channel::bounded(1);

    mqtt_client
        .subscribe("hello/world", QoS::AtLeastOnce)
        .unwrap();

    let sleep_time = Duration::from_millis(100);

    thread::spawn(move || {
        for i in 0..COUNT {
            let payload = format!("publish {}", i);
            thread::sleep(sleep_time);

            mqtt_client
                .publish("hello/world", QoS::AtLeastOnce, payload)
                .unwrap();
        }

        thread::sleep(sleep_time * 10);
        done_tx.send(true).unwrap();
    });

    handle_notifications(notifications, done_rx);
}