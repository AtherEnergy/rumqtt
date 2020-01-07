use futures::stream::StreamExt;
use rumqtt::{MqttClient, MqttOptions, QoS, ReconnectOptions};
use std::time::Duration;
use tokio::time::delay_for;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    let reconnect_options = ReconnectOptions::Always(5);
    let mqtt_options = MqttOptions::new("test-id", "localhost", 1883)
        .set_keep_alive(10)
        .set_reconnect_opts(reconnect_options);

    let (mut mqtt_client, mut notifications) = MqttClient::start(mqtt_options).await.unwrap();
    let thread = tokio::spawn(async move {
        delay_for(Duration::from_secs(2)).await;

        mqtt_client
            .publish("hello/world", QoS::AtLeastOnce, true, "test")
            .await
            .unwrap();

        delay_for(Duration::from_secs(300)).await;
    });

    while let Some(notification) = notifications.next().await {
        println!("{:?}", notification)
    }
    thread.await.unwrap();
}
