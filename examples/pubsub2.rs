use futures::stream::StreamExt;
use rumqtt::{MqttClient, MqttOptions, QoS};
use std::time::Duration;
use tokio::time::delay_for;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    let mqtt_options = MqttOptions::new("test-pubsub2", "127.0.0.1", 1883).set_keep_alive(10);
    let (mut mqtt_client, mut notifications) = MqttClient::start(mqtt_options).await.unwrap();

    //mqtt_client.subscribe("hello/world", QoS::ExactlyOnce).await.unwrap();

    let thread = tokio::spawn(async move {
        for i in 0..100 {
            let payload = format!("publish {}", i);
            delay_for(Duration::from_secs(1)).await;
            mqtt_client
                .publish("hello/world", QoS::AtMostOnce, false, payload)
                .await
                .unwrap();
        }
    });

    while let Some(notification) = notifications.next().await {
        println!("{:?}", notification)
    }
    thread.await.unwrap();
}
