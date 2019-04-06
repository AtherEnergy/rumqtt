use rumqtt::{MqttClient, MqttOptions, QoS, ReconnectOptions};
fn main() {
    pretty_env_logger::init();
    //let broker = "prod-mqtt-broker.atherengineering.in";
    let broker = "test.mosquitto.org";

    let opts = MqttOptions::builder()
        .client_id("test-pubsub2")
        .host(broker)
        .port(1883)
        .keep_alive(10)
        .reconnect_opts(ReconnectOptions::Always(10))
        .clean_session(false)
        .build();

    let (mut mqtt_client, notifications) = MqttClient::start(opts).unwrap();
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

use std::{thread, time::Duration};
