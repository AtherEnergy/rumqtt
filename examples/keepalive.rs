use rumqtt::{MqttClient, MqttOptions, ReconnectOptions};

fn main() {
    pretty_env_logger::init();
    let reconnect_options = ReconnectOptions::Always(5);
    let opts = MqttOptions::builder()
        .client_id("test-id")
        .host("test.mosquitto.org")
        .port(1883)
        .keep_alive(10)
        .reconnect_opts(reconnect_options)
        .build();

    let (_client, notifications) = MqttClient::start(opts).unwrap();

    for notification in notifications {
        println!("{:?}", notification)
    }
}
