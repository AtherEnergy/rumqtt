extern crate pretty_env_logger;
extern crate rumqtt;
extern crate tokio;
extern crate futures;
#[macro_use]
extern crate serde_derive;

use rumqtt::{MqttClient, MqttOptions, ReconnectOptions, Proxy};
use std::net::SocketAddr;
use tokio::net::TcpStream;
use futures::future;
use tokio::codec::LinesCodec;
use futures::future::Future;
use futures::sink::Sink;
use futures::stream::Stream;
use tokio::codec::Decoder;
use tokio::io::AsyncRead;

#[derive(Deserialize, Debug)]
struct Config {
    proxy_host: String,
    proxy_port: u16,
    main_host: String,
    main_port: u16,
    proxy_auth: String
}


fn main() {
    pretty_env_logger::init();
    let config: Config = envy::from_env().unwrap();

    let reconnect_options = ReconnectOptions::Never;
    let proxy = Proxy::HttpConnect(config.proxy_host, config.proxy_port, config.proxy_auth);

    let id = "test-id";
    let host = "prod-mqtt-broker.atherengineering.in";
    let port = 1883;

    let mqtt_options = MqttOptions::new(id, host, port);

    let mqtt_options = mqtt_options
        .set_keep_alive(10)
        .set_reconnect_opts(reconnect_options)
        .set_proxy(proxy);

    let (_client, notifications) = MqttClient::start(mqtt_options).unwrap();

    for notification in notifications {
        println!("{:?}", notification)
    }
}