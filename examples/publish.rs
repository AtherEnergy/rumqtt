extern crate rumqtt;
extern crate loggerv;

use std::thread;
use std::time::Duration;

use rumqtt::{MqttOptions, ReconnectOptions, MqttClient, QoS};

fn main() {
    loggerv::init_with_verbosity(1).unwrap();
    let mqtt_opts = MqttOptions::new("rumqtt-core", "127.0.0.1:1889")
                                .set_reconnect_opts(ReconnectOptions::Always(10));

    let (mut client, receiver) = MqttClient::start(mqtt_opts);

    for i in 0..100000 {
        if let Err(e) = client.publish("hello/world", QoS::AtLeastOnce, vec![1, 2, 3]) {
            println!("{:?}", e);
        }
        // thread::sleep(Duration::new(1, 0));
    }

    thread::sleep(Duration::new(60, 0));
}