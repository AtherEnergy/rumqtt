extern crate pretty_env_logger;
extern crate rumqtt;
extern crate tokio;
extern crate futures;
#[macro_use]
extern crate serde_derive;

use rumqtt::{MqttClient, MqttOptions, ReconnectOptions};
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

fn lookup_ipv4(host: &str, port: u16) -> SocketAddr {
    use std::net::ToSocketAddrs;

    let addrs = (host, port).to_socket_addrs().unwrap();
    for addr in addrs {
        if let SocketAddr::V4(_) = addr {
            return addr;
        }
    }

    unreachable!("Cannot lookup address");
}

fn connect_future(proxy_host: &str, proxy_port: u16, main_host: &str, main_port: u16, proxy_auth: &str) -> impl Future<Item = TcpStream>{
    let codec = LinesCodec::new();
    let host = main_host.to_string() + &format!(":{}", main_port);
    let socket = lookup_ipv4(proxy_host, proxy_port);

    let connect = format!("CONNECT {} HTTP/1.1\r\nHost: {}\r\nProxy-Authorization: {}\r\n\r\n", host, host, proxy_auth);
    println!("{}", connect);

    TcpStream::connect(&socket)
        .and_then(|mut tcp| {
            let framed = tcp.framed(codec);
            future::ok(framed)
        })
        .and_then(|f| f.send(connect))
        .and_then(|f| f.into_future().map_err(|(e, f)| {
            e
        }))
        .and_then(|(s, f)| {
            println!("{:?}", s);
            f.into_future().map_err(|(e, f)| e)
        })
        .and_then(|(s, f)| {
            println!("{:?}", s);
            let stream = f.into_inner();
            future::ok(stream)
        })
        .and_then(|s| {
            future::ok(s)
        })
}

fn main() {
    pretty_env_logger::init();
    let config: Config = envy::from_env().unwrap();
    let connect_future = connect_future(&config.proxy_host, config.proxy_port, &config.main_host, config.main_port, &config.proxy_auth);

    let reconnect_options = ReconnectOptions::Never;
    let mqtt_options = MqttOptions::new("test-id", "127.0.0.1", 1883)
        .set_keep_alive(10)
        .set_reconnect_opts(reconnect_options);

    let (_client, notifications) = MqttClient::start(mqtt_options).unwrap();

    for notification in notifications {
        println!("{:?}", notification)
    }
}