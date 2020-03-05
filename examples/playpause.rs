use futures::stream::StreamExt;
use rumqtt::{MqttClient, MqttOptions, QoS};
use std::time::Duration;
use tokio::time::delay_for;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    let mqtt_options = MqttOptions::new("test-id", "127.0.0.1", 1883).set_keep_alive(10);

    let (mut mqtt_client, mut notifications) = MqttClient::start(mqtt_options).await.unwrap();

    mqtt_client.subscribe("hello/world", QoS::AtLeastOnce).await.unwrap();

    let mut c1 = mqtt_client.clone();
    let mut c2 = mqtt_client.clone();

    let thread1 = tokio::spawn(async move {
        let dur = Duration::new(1, 0);

        for i in 0..100 {
            let payload = format!("publish {}", i);
            delay_for(dur).await;
            c1.publish("hello/world", QoS::AtLeastOnce, false, payload).await.unwrap();
        }
    });

    let thread2 = tokio::spawn(async move {
        let dur = Duration::new(5, 0);

        for i in 0..100 {
            if i % 2 == 0 {
                c2.pause().await.unwrap();
            } else {
                c2.resume().await.unwrap();
            }

            delay_for(dur).await;
        }
    });

    while let Some(notification) = notifications.next().await {
        println!("{:?}", notification)
    }
    thread1.await.unwrap();
    thread2.await.unwrap();
}
