use rumqtt::{MqttClient, MqttOptions, ReconnectOptions, QoS};
use std::thread;
use std::time::Duration;

fn main() {
    pretty_env_logger::init();
    let reconnect_options = ReconnectOptions::Always(5);
    let mqtt_options = MqttOptions::new("test-id", "localhost", 1883)
        .set_keep_alive(10)
        .set_reconnect_opts(reconnect_options);

    let (mut mqtt_client, notifications) = MqttClient::start(mqtt_options).unwrap();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(2));

        mqtt_client
            .publish("hello/world", QoS::AtLeastOnce, true, "test")
            .unwrap();

        thread::sleep(Duration::from_secs(300));
    });

    for notification in notifications {
        println!("{:?}", notification)
    }
}
