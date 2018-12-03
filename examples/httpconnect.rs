extern crate futures;
extern crate pretty_env_logger;
extern crate rumqtt;
extern crate tokio;
#[macro_use]
extern crate serde_derive;

use futures::{
    future::{self, Future},
    sink::Sink,
    stream::Stream,
};
use rumqtt::{MqttClient, MqttOptions, Proxy, ReconnectOptions};
use std::net::SocketAddr;
use tokio::{
    codec::{Decoder, LinesCodec},
    io::AsyncRead,
    net::TcpStream,
};

#[derive(Deserialize, Debug)]
struct Config {
    proxy_host: String,
    proxy_port: u16,
    main_host: String,
    main_port: u16,
    proxy_auth: String,
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

    let mqtt_options = mqtt_options.set_keep_alive(10)
                                   .set_reconnect_opts(reconnect_options)
                                   .set_proxy(proxy);

    let (_client, notifications) = MqttClient::start(mqtt_options).unwrap();

    for notification in notifications {
        println!("{:?}", notification)
    }
}
