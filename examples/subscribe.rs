extern crate rumqtt;
extern crate loggerv;

use std::thread;
use std::time::Duration;

use rumqtt::{MqttOptions, ReconnectOptions, MqttClient, QoS};

fn main() {
    loggerv::init_with_verbosity(1).unwrap();
    let mqtt_opts = MqttOptions::new("rumqtt-core", "test.mosquitto.org:1883").unwrap()

                                .set_reconnect_opts(ReconnectOptions::AfterFirstSuccess(10));

    let (mut client, receiver) = MqttClient::start(mqtt_opts);

    client.subscribe(vec![("hello/world", QoS::AtLeastOnce)]);

    thread::spawn(move || {
        for msg in receiver {
            println!("Received = {:?}", msg);
        }
    });

    for i in 0..100 {
        client.publish("hello/world", QoS::AtLeastOnce, vec![1, 2, 3]);
        //thread::sleep(Duration::new(1, 0));
    }

    thread::sleep(Duration::new(60, 0));
}