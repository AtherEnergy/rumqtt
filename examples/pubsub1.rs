use rumqtt::{MqttClient, MqttOptions, QoS, ReconnectOptions};
use std::{thread, time::Duration};

fn main() {
    pretty_env_logger::init();
    //let broker = "prod-mqtt-broker.atherengineering.in";
    let broker = "test.mosquitto.org";
    let port = 1883;

    let reconnection_options = ReconnectOptions::Always(10);
    let mut opts = MqttOptions::new("test-pubsub2", broker, port);
    opts.set_keep_alive(10);
    opts.set_reconnect_opts(reconnection_options);
    opts.set_clean_session(false);

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
