use futures::{channel::oneshot, future::FutureExt, select, stream::StreamExt};
use rumqtt::{MqttClient, MqttOptions, QoS};
use std::time::Duration;
use tokio::time::delay_for;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    let mqtt_options = MqttOptions::new("test-id", "127.0.0.1", 1883).set_keep_alive(30);
    let (mut mqtt_client, mut notifications) = MqttClient::start(mqtt_options).await.unwrap();
    let (done_tx, done_rx) = oneshot::channel();

    mqtt_client.subscribe("hello/world", QoS::AtLeastOnce).await.unwrap();

    let sleep_time = Duration::from_millis(100);

    let thread = tokio::spawn(async move {
        for i in 0..1000 {
            let payload = format!("publish {}", i);
            delay_for(sleep_time).await;

            mqtt_client
                .publish("hello/world", QoS::AtLeastOnce, false, payload)
                .await
                .unwrap();
        }

        delay_for(sleep_time * 10).await;
        done_tx.send(()).unwrap();
    });

    let mut done_rx = done_rx.fuse();
    loop {
        select! {
            notification = notifications.next() => {
                println!("{:?}", notification)
            },
            res = done_rx => {
                let () = res.unwrap();
                break;
            },
        }
    }
    thread.await.unwrap();
}
