use futures::stream::StreamExt;
use rumqtt::{MqttClient, MqttOptions, QoS};
use std::time::Duration;
use tokio::time::delay_for;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let client_id = "tls-test".to_owned();
    let ca = include_bytes!("tlsfiles/ca-chain.cert.pem").to_vec();
    let client_cert = include_bytes!("tlsfiles/bike1.cert.pem").to_vec();
    let client_key = include_bytes!("tlsfiles/bike1.key.pem").to_vec();

    let mqtt_options = MqttOptions::new(client_id, "localhost", 8883)
        .set_ca(ca)
        .set_client_auth(client_cert, client_key)
        .set_keep_alive(10);

    let (mut mqtt_client, mut notifications) = MqttClient::start(mqtt_options).await.unwrap();
    let topic = "hello/world";

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
}
