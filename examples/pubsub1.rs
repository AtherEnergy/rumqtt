use futures::stream::StreamExt;
use rumqtt::{MqttClient, MqttOptions, QoS, ReconnectOptions};
use std::time::Duration;
use tokio::time::delay_for;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    //let broker = "prod-mqtt-broker.atherengineering.in";
    let broker = "test.mosquitto.org";
    let port = 1883;

    let reconnection_options = ReconnectOptions::Always(10);
    let mqtt_options = MqttOptions::new("test-pubsub2", broker, port)
        .set_keep_alive(10)
        .set_inflight(3)
        .set_request_channel_capacity(3)
        .set_reconnect_opts(reconnection_options)
        .set_clean_session(false);

    let (mut mqtt_client, mut notifications) = MqttClient::start(mqtt_options).await.unwrap();
    mqtt_client.subscribe("hello/world", QoS::AtLeastOnce).await.unwrap();

    let thread = tokio::spawn(async move {
        for i in 0..100 {
            let payload = format!("publish {}", i);
            delay_for(Duration::from_millis(1000)).await;
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
