extern crate envy;
extern crate pretty_env_logger;
extern crate rumqtt;
#[macro_use]
extern crate serde_derive;

use rumqtt::{ConnectionMethod, MqttClient, MqttOptions, QoS, SecurityOptions};
use std::thread;


// NOTES:
// ---------
// Proive necessary stuff from environment variables
// RUST_LOG=rumqtt=debug PROJECT=ABC ID=DEF REGISTRY=GHI cargo run --example gcloud

#[derive(Deserialize, Debug)]
struct Config {
    project: String,
    id: String,
    registry: String,
}

fn main() {
    pretty_env_logger::init();
    let config: Config = envy::from_env().unwrap();

    let client_id = "projects/".to_owned()
        + &config.project
        + "/locations/us-central1/registries/"
        + &config.registry
        + "/devices/"
        + &config.id;

    let security_options = SecurityOptions::GcloudIot((
        config.project,
        include_bytes!("gcloudfiles/rsa_private.der").to_vec(),
        60,
    ));

    let ca = include_bytes!("gcloudfiles/roots.pem").to_vec();
    let connection_method = ConnectionMethod::Tls(ca, None);

    let mqtt_options = MqttOptions::new(client_id, "mqtt.googleapis.com", 8883)
        .set_keep_alive(10)
        .set_connection_method(connection_method)
        .set_security_opts(security_options);

    let (mut mqtt_client, notifications) = MqttClient::start(mqtt_options);
    let topic = "/devices/".to_owned() + &config.id + "/events/imu";

    thread::spawn(move || {
        for i in 0..100 {
            let payload = format!("publish {}", i);
            thread::sleep_ms(1000);
            mqtt_client
                .publish(topic.clone(), QoS::AtLeastOnce, payload)
                .unwrap();
        }
    });

    for notification in notifications {
        println!("{:?}", notification)
    }
}
