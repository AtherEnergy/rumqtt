extern crate rumqtt;

use rumqtt::{MqttOptions, MqttClient, QoS};

fn main() {
    let mqtt_opts = MqttOptions::new("rumqtt-core", "localhost:1883");

    let mut client = MqttClient::start(mqtt_opts);

    for i in 0..100 {
        client.publish("hello/world", QoS::AtLeastOnce, vec![1, 2, 3]);
    }
}