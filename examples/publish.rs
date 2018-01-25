extern crate rumqtt;
extern crate pretty_env_logger;

use std::thread;
use std::time::Duration;

use rumqtt::{MqttOptions, ReconnectOptions, SecurityOptions, MqttClient, QoS};

fn main() {
    pretty_env_logger::init().unwrap();

    let mqtt_opts = MqttOptions::new("pub-1", "localhost:1883").unwrap()
                                .set_reconnect_opts(ReconnectOptions::AfterFirstSuccess(10))
                                .set_clean_session(false)
                                .set_security_opts(SecurityOptions::None);
    

    let (mut client, receiver) = MqttClient::start(mqtt_opts);

    thread::spawn(||{
        for i in receiver {
            println!("{:?}", i);
        }
    });

    for i in 0..100 {
        if let Err(e) = client.publish("/devices/RAVI-MAC/events/imu", QoS::AtLeastOnce, vec![1, 2, 3]) {
            println!("Publish error = {:?}", e);
        }
        thread::sleep(Duration::new(1, 0));
    }
    

    thread::sleep(Duration::new(60, 0));
}