extern crate rumqtt;
extern crate pretty_env_logger;
extern crate rand;

use rand::{thread_rng, Rng};

use std::thread;
use std::time::Duration;

use rumqtt::{MqttOptions, ReconnectOptions, SecurityOptions, MqttClient, QoS};

fn main() {
    pretty_env_logger::init().unwrap();

    let mqtt_opts = MqttOptions::new("pub-1", "127.0.0.1:9883").unwrap()
                                .set_reconnect_opts(ReconnectOptions::AfterFirstSuccess(10))
                                .set_clean_session(false)
                                .set_security_opts(SecurityOptions::None);
    

    {
        let (mut client, receiver) = MqttClient::start(mqtt_opts);
    
        thread::spawn(||{
            for i in receiver {
                println!("{:?}", i);
            }
        });
    
        for _ in 0..20 {
            // create payload of size 0-100K bytes
            let x: usize = thread_rng().gen_range(0, 256 * 1024);
            let mut payload: Vec<usize> = (0..x).collect();
            let mut payload: Vec<u8> = payload.into_iter().map(|v: usize| (v % 255) as u8).collect();
            thread_rng().shuffle(&mut payload);

            if let Err(e) = client.publish("/devices/RAVI-MAC/events/imu", QoS::AtLeastOnce, payload) {
                println!("Publish error = {:?}", e);
            }
            thread::sleep(Duration::from_secs(1));
        }

        thread::sleep(Duration::from_secs(360));
    }

    thread::sleep(Duration::from_secs(60));
}
