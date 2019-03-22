use rumqtt::{MqttClient, MqttOptions, ReconnectOptions};

fn main() {
    pretty_env_logger::init();
    let reconnect_options = ReconnectOptions::Always(5);
    let mut opts = MqttOptions::new("test-id", "test.mosquitto.org", 1883);
    opts.set_keep_alive(10);
    opts.set_reconnect_opts(reconnect_options);

    let (_client, notifications) = MqttClient::start(opts).unwrap();

    for notification in notifications {
        println!("{:?}", notification)
    }
}
