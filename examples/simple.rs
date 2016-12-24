extern crate tokio_mqtt;
extern crate mqtt3;
extern crate futures;
extern crate tokio_core as core;
extern crate tokio_proto as proto;
extern crate tokio_service as service;

use futures::Future;
use proto::TcpClient;
use service::Service;

use tokio_mqtt::client::MqttClientProto;
use mqtt3::{Packet, Protocol,Connect, LastWill, QoS};

use std::net::{SocketAddr, ToSocketAddrs};

fn lookup_ipv4<A: ToSocketAddrs>(addr: A) -> SocketAddr {
        let addrs = addr.to_socket_addrs().expect("Conversion Failed");
        for addr in addrs {
            if let SocketAddr::V4(_) = addr {
                return addr;
            }
        }
        unreachable!("Cannot lookup address");
}


fn main() {
    let mut event_loop = core::reactor::Core::new().unwrap();
    let handle = event_loop.handle();

    let addr = lookup_ipv4("test.mosquitto.org:1883");
    let test = TcpClient::new(MqttClientProto)
        .connect(&addr, &handle.clone())
        .and_then(|mut client| {
            let connect = Packet::Connect(Box::new(Connect {
            protocol: Protocol::MQTT(4),
            keep_alive: 10,
            client_id: "tokio-mqtt".to_owned(),
            clean_session: true,
            last_will: None,
            username: None,
            password: None,
            }));

            client.call(connect)
        })
        .map(|res| println!("res: {:?}", res));

    event_loop.run(test).unwrap();
}