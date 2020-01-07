extern crate pretty_env_logger;
extern crate rumqtt;
//use rumqtt::{MqttClient, MqttOptions};
//use std::thread;
//use std::time::Duration;
//use tokio::time::delay_for;

#[tokio::main]
async fn main() {
    // pretty_env_logger::init();
    // let mqtt_options = MqttOptions::new("test-id-1", "localhost", 1883);

    // let (mut mqtt_client, mut notifications) = MqttClient::start(mqtt_options);

    // tokio::spawn(async move {
    //     // let mqtt_options = MqttOptions::new("test-id-2", "localhost", 1883).set_keep_alive(120);
    //     // mqtt_client.reconnect(mqtt_options).unwrap();
    // });

    // while let Some(notification) = notifications.next().await {
    //     println!("{:?}", notification)
    // }
    // thread.await.unwrap();
}
