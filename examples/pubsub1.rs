use rumqtt::{MqttClient, MqttOptions, QoS, ReconnectOptions};
use std::{thread, time::Duration};

fn main() {
    pretty_env_logger::init();
    let broker = "localhost";
    let port = 1883;

    let reconnection_options = ReconnectOptions::Always(10);
    let mqtt_options = MqttOptions::new("test-pubsub2", broker, port)
                                    .set_keep_alive(10)
                                    .set_reconnect_opts(reconnection_options)
                                    .set_clean_session(false);

    let (mut mqtt_client, notifications) = MqttClient::start(mqtt_options).unwrap();
    mqtt_client.subscribe("hello/world", QoS::AtLeastOnce).unwrap();

    thread::spawn(move || {
        for i in 0..100 {
            let payload = format!("publish {}", i);
            thread::sleep(Duration::from_secs(1));
            mqtt_client.publish("hello/world", QoS::AtLeastOnce, false, payload).unwrap();
        }
    });

    for notification in notifications {
        println!("{:?}", notification)
    }
}
