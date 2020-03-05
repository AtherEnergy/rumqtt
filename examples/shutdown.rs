use futures::stream::StreamExt;
use rumqtt::{MqttClient, MqttOptions, QoS};
use std::time::Duration;
use tokio::time::delay_for;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    let mqtt_options = MqttOptions::new("test-id-1", "localhost", 1883).set_keep_alive(10);

    let (mut mqtt_client, mut notifications) = MqttClient::start(mqtt_options).await.unwrap();

    let thread = tokio::spawn(async move {
        delay_for(Duration::from_secs(5)).await;

        for i in 1..11 {
            let payload = format!("publish {}", i);
            delay_for(Duration::from_millis(100)).await;
            mqtt_client
                .publish("hello/world", QoS::AtLeastOnce, false, payload)
                .await
                .unwrap();
        }

        mqtt_client.shutdown().await.unwrap();

        for i in 11..21 {
            let payload = format!("publish {}", i);
            delay_for(Duration::from_millis(100)).await;
            mqtt_client
                .publish("hello/world", QoS::AtLeastOnce, false, payload)
                .await
                .unwrap();
        }
    });

    while let Some(notification) = notifications.next().await {
        println!("{:?}", notification)
    }
    thread.await.unwrap();
}
