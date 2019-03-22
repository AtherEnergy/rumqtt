use rumqtt::{ConnectionMethod, MqttClient, MqttOptions, QoS, SecurityOptions};
use serde_derive::Deserialize;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::{thread, time::Duration};

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

fn main() -> Result<(), io::Error> {
    pretty_env_logger::init();
    let config: Config = envy::from_env().unwrap();

    let client_id = "projects/".to_owned()
        + &config.project
        + "/locations/us-central1/registries/"
        + &config.registry
        + "/devices/"
        + &config.id;

    let mut rsa_private = vec!();
    File::open(Path::new("../../certs/rsa_private.der")).and_then(|mut f| f.read_to_end(&mut rsa_private))?;
    let security_options = SecurityOptions::GcloudIot(config.project, rsa_private, 60);

    let mut ca = vec!();
    File::open(Path::new("../../certs/roots.pem")).and_then(|mut f| f.read_to_end(&mut ca))?;
    let connection_method = ConnectionMethod::Tls(ca, None);

    let mut opts = MqttOptions::new(client_id, "mqtt.googleapis.com", 8883);
    opts.set_keep_alive(10);
    opts.set_connection_method(connection_method);
    opts.set_security_opts(security_options);

    let (mut mqtt_client, notifications) = MqttClient::start(opts).unwrap();
    let topic = "/devices/".to_owned() + &config.id + "/events/imu";

    thread::spawn(move || {
        for i in 0..100 {
            let payload = format!("publish {}", i);
            thread::sleep(Duration::from_secs(1));
            mqtt_client.publish(topic.clone(), QoS::AtLeastOnce, false, payload).unwrap();
        }
    });

    for notification in notifications {
        println!("{:?}", notification)
    }
    Ok(())
}
