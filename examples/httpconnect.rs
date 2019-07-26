use rumqtt::{MqttClient, MqttOptions, Proxy, QoS, ReconnectOptions};
use serde_derive::Deserialize;

use std::thread;
use std::time::Duration;

#[derive(Deserialize, Debug)]
struct Config {
    proxy_host: String,
    proxy_port: u16,
    main_host: String,
    main_port: u16,
}

fn main() {
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

    let (mut mqtt_client, notifications) = MqttClient::start(mqtt_options, None).unwrap();

    mqtt_client.subscribe("hello/world", QoS::AtLeastOnce).unwrap();

    thread::spawn(move || {
        for i in 0..100 {
            let payload = format!("publish {}", i);
            thread::sleep(Duration::from_millis(100));
            mqtt_client.publish("hello/world", QoS::AtLeastOnce, false, payload).unwrap();
        }
    });

    for notification in notifications {
        println!("{:?}", notification)
    }
}
