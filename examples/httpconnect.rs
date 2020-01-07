use futures::stream::StreamExt;
use rumqtt::{MqttClient, MqttOptions, Proxy, QoS, ReconnectOptions};
use serde_derive::Deserialize;
use std::time::Duration;
use tokio::time::delay_for;

#[derive(Deserialize, Debug)]
struct Config {
    proxy_host: String,
    proxy_port: u16,
    main_host: String,
    main_port: u16,
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    let config: Config = envy::from_env().unwrap();
    let key = include_bytes!("tlsfiles/server.key.pem");

    let reconnect_options = ReconnectOptions::AfterFirstSuccess(10);
    let proxy = Proxy::HttpConnect(config.proxy_host, config.proxy_port, key.to_vec(), 40);

    let id = "http-connect-test";
    let mqtt_options = MqttOptions::new(id, config.main_host, config.main_port);

    let mqtt_options = mqtt_options
        .set_keep_alive(10)
        .set_reconnect_opts(reconnect_options)
        .set_proxy(proxy);

    let (mut mqtt_client, mut notifications) = MqttClient::start(mqtt_options).await.unwrap();

    mqtt_client.subscribe("hello/world", QoS::AtLeastOnce).await.unwrap();

    let thread = tokio::spawn(async move {
        for i in 0..100 {
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
