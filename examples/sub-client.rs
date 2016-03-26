extern crate mqtt;
#[macro_use]
extern crate log;
extern crate env_logger;
extern crate clap;
extern crate uuid;

use std::net::TcpStream;
use std::io::Write;
use std::str;

use clap::{App, Arg};

use uuid::Uuid;

use mqtt::{Encodable, Decodable, QualityOfService};
use mqtt::packet::*;
use mqtt::control::variable_header::ConnectReturnCode;
use mqtt::TopicFilter;

fn generate_client_id() -> String {
    format!("/MQTT/rust/{}", Uuid::new_v4().to_simple_string())
}

fn main() {
    env_logger::init().unwrap();

    let matches = App::new("sub-client")
                      .author("Y. T. Chung <zonyitoo@gmail.com>")
                      .arg(Arg::with_name("HOST")
                               .short("h")
                               .long("host")
                               .takes_value(true)
                               .required(true)
                               .help("MQTT host"))
                      .arg(Arg::with_name("SUBSCRIBE")
                               .short("s")
                               .long("subscribe")
                               .takes_value(true)
                               .multiple(true)
                               .required(true)
                               .help("Channel filter to subscribe"))
                      .arg(Arg::with_name("USER_NAME")
                               .short("u")
                               .long("username")
                               .takes_value(true)
                               .help("Login user name"))
                      .arg(Arg::with_name("PASSWORD")
                               .short("p")
                               .long("password")
                               .takes_value(true)
                               .help("Password"))
                      .arg(Arg::with_name("CLIENT_ID")
                               .short("i")
                               .long("client-identifier")
                               .takes_value(true)
                               .help("Client identifier"))
                      .get_matches();

    let server_addr = matches.value_of("HOST").unwrap();
    let client_id = matches.value_of("CLIENT_ID")
                           .map(|x| x.to_owned())
                           .unwrap_or_else(generate_client_id);
    let channel_filters: Vec<(TopicFilter, QualityOfService)> =
        matches.values_of("SUBSCRIBE")
               .unwrap()
               .iter()
               .map(|c| {
                   (TopicFilter::new_checked(c.to_string()).unwrap(),
                    QualityOfService::Level0)
               })
               .collect();

    print!("Connecting to {:?} ... ", server_addr);
    let mut stream = TcpStream::connect(server_addr).unwrap();
    println!("Connected!");

    println!("Client identifier {:?}", client_id);
    let mut conn = ConnectPacket::new("MQTT".to_owned(), client_id.to_owned());
    conn.set_clean_session(true);
    conn.set_keep_alive(5);
    let mut buf = Vec::new();
    conn.encode(&mut buf).unwrap();
    stream.write_all(&buf[..]).unwrap();

    let connack = ConnackPacket::decode(&mut stream).unwrap();
    trace!("CONNACK {:?}", connack);

    if connack.connect_return_code() != ConnectReturnCode::ConnectionAccepted {
        panic!("Failed to connect to server, return code {:?}",
               connack.connect_return_code());
    }

    // const CHANNEL_FILTER: &'static str = "typing-speed-test.aoeu.eu";
    println!("Applying channel filters {:?} ...", channel_filters);
    let sub = SubscribePacket::new(10, channel_filters);
    let mut buf = Vec::new();
    sub.encode(&mut buf).unwrap();
    stream.write_all(&buf[..]).unwrap();

    loop {
        let packet = match VariablePacket::decode(&mut stream) {
            Ok(pk) => pk,
            Err(err) => {
                error!("Error in receiving packet {:?}", err);
                continue;
            }
        };
        trace!("PACKET {:?}", packet);

        match &packet {
            &VariablePacket::SubackPacket(ref ack) => {
                if ack.packet_identifier() != 10 {
                    panic!("SUBACK packet identifier not match");
                }

                println!("Subscribed!");
                break;
            }
            _ => {}
        }
    }

    loop {
        let packet = match VariablePacket::decode(&mut stream) {
            Ok(pk) => pk,
            Err(err) => {
                error!("Error in receiving packet {}", err);
                continue;
            }
        };
        trace!("PACKET {:?}", packet);

        match &packet {
            &VariablePacket::PingreqPacket(..) => {
                let pingresp = PingrespPacket::new();
                info!("Sending Ping response {:?}", pingresp);
                pingresp.encode(&mut stream).unwrap();
            }
            &VariablePacket::PublishPacket(ref publ) => {
                let msg = match str::from_utf8(&publ.payload()[..]) {
                    Ok(msg) => msg,
                    Err(err) => {
                        error!("Failed to decode publish message {:?}", err);
                        continue;
                    }
                };
                println!("PUBLISH ({}): {}", publ.topic_name(), msg);
            }
            &VariablePacket::DisconnectPacket(..) => {
                error!("Received disconnect packet from broker");
                break;
            }
            _ => {}
        }
    }
}
