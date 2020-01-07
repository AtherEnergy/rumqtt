use futures::stream::StreamExt;
use rumqtt::{MqttClient, MqttOptions, QoS, SecurityOptions};
use serde_derive::Deserialize;
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
    time::Duration,
};
use tokio::time::delay_for;

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

#[tokio::main]
async fn main() -> Result<(), io::Error> {
    pretty_env_logger::init();
    let config: Config = envy::from_env().unwrap();

    let client_id = "projects/".to_owned()
        + &config.project
        + "/locations/us-central1/registries/"
        + &config.registry
        + "/devices/"
        + &config.id;

    let mut rsa_private = vec![];
    File::open(Path::new("../../certs/rsa_private.der")).and_then(|mut f| f.read_to_end(&mut rsa_private))?;
    let security_options = SecurityOptions::GcloudIot(config.project, rsa_private, 60);

    let mut ca = vec![];
    File::open(Path::new("../../certs/roots.pem")).and_then(|mut f| f.read_to_end(&mut ca))?;

    let mqtt_options = MqttOptions::new(client_id, "mqtt.googleapis.com", 8883)
        .set_ca(ca)
        .set_keep_alive(10)
        .set_security_opts(security_options);

    let (mut mqtt_client, mut notifications) = MqttClient::start(mqtt_options).await.unwrap();
    let topic = "/devices/".to_owned() + &config.id + "/events/imu";

    let thread = tokio::spawn(async move {
        for i in 0..100 {
            let payload = format!("publish {}", i);
            delay_for(Duration::from_secs(1)).await;
            mqtt_client
                .publish(topic.clone(), QoS::AtLeastOnce, false, payload)
                .await
                .unwrap();
        }
    });

    while let Some(notification) = notifications.next().await {
        println!("{:?}", notification)
    }
    thread.await.unwrap();
    Ok(())
}
