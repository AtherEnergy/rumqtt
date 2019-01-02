use rumqtt::{MqttClient, MqttOptions, ReconnectOptions};

fn main() {
    pretty_env_logger::init();
    let reconnect_options = ReconnectOptions::Always(5);
    let mqtt_options = MqttOptions::new("test-id", "test.mosquitto.org", 1883)
        .set_keep_alive(10)
        .set_reconnect_opts(reconnect_options);

    let (_client, notifications) = MqttClient::start(mqtt_options).unwrap();

    for notification in notifications {
        println!("{:?}", notification)
    }
}
