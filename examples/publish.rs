extern crate rumqtt;
extern crate loggerv;

use std::thread;
use std::time::Duration;

use rumqtt::{MqttOptions, MqttClient, QoS};

fn main() {
    loggerv::init_with_verbosity(1).unwrap();
    let mqtt_opts = MqttOptions::new("rumqtt-core", "127.0.0.1:1883").set_reconnect_after(5);

    let mut client = MqttClient::start(mqtt_opts);

    for i in 0..100 {
        client.publish("hello/world", QoS::AtLeastOnce, vec![1, 2, 3]);
        thread::sleep(Duration::new(2, 0));
    }
}