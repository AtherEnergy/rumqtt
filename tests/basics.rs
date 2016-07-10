extern crate rumqtt;
#[macro_use]
extern crate log;
extern crate env_logger;


use rumqtt::{ClientOptions, SslContext, QoS};
use std::thread;
use std::time::Duration;

//#[test]
fn tls_connect() {
    // USAGE: RUST_LOG=rumqtt cargo test -- --nocapture
    env_logger::init().unwrap();
    let ca = "./tests/ca.crt";
    //let cert = "./tests/scooter.crt";
    //let key = "./tests/scooter.key";
    let ssl = SslContext::with_ca(ca).expect("#Ssl context error");
    //let ssl = SslContext::with_cert_key_and_ca(cert, key, ca).expect("#Ssl context error");
    //let ssl = SslContext::with_cert_and_key(cert, key).expect("#Ssl context error");

    let mut client_options = ClientOptions::new();

    // Specify client connection opthons and which broker to connect to
    let proxy_client = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .set_tls(ssl)
                                    .connect("localhost:1883");

    // Connects to a broker and returns a `Publisher` and `Subscriber`
    let (_, _) = proxy_client.start().expect("Coudn't start");
    thread::sleep(Duration::new(120, 0));
}   

// #[test]
fn pingreq_reconnect_test() {
    // USAGE: RUST_LOG=rumqtt cargo test -- --nocapture
    env_logger::init().unwrap();

    let mut client_options = ClientOptions::new();

    // Specify client connection opthons and which broker to connect to
    let proxy_client = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .connect("localhost:1883");

    // Connects to a broker and returns a `Publisher` and `Subscriber`
    let (_, _) = proxy_client.start().expect("Coudn't start");

    thread::sleep(Duration::new(120, 0));
}

/// Test publishes along with ping requests and responses
/// Observe if the boker is getting ping requests with in keep_alive time
/// Add handling in client if pingresp isn't received for a ping request
// #[test]
fn publish_qos0_test() {
    env_logger::init().unwrap();

    let mut client_options = ClientOptions::new();

    // Specify client connection opthons and which broker to connect to
    let proxy_client = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .connect("localhost:1883");

    // Connects to a broker and returns a `Publisher` and `Subscriber`
    let (publisher, _) = proxy_client.start().expect("Coudn't start");

    for i in 0..100 {
        let payload = format!("{}. hello rust", i);
        publisher.publish("hello/rust", QoS::Level0, payload.into_bytes());
        thread::sleep(Duration::new(1, 0));
    }

    thread::sleep(Duration::new(120, 0));
}

//#[test]
fn publish_qos1_test() {
    env_logger::init().unwrap();

    let mut client_options = ClientOptions::new();

    // Specify client connection opthons and which broker to connect to
    let proxy_client = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .set_pub_q_len(10)
                                    .connect("localhost:1883");

    // Connects to a broker and returns a `Publisher` and `Subscriber`
    let (publisher, _) = proxy_client.start().expect("Coudn't start");

    for i in 0..100 {
        let payload = format!("{}. hello rust", i);
        publisher.publish("hello/rust", QoS::Level1, payload.into_bytes());
        thread::sleep(Duration::new(1, 0));
    }

    thread::sleep(Duration::new(120, 0));
}

//#[test]
fn subscribe_test() {
    env_logger::init().unwrap();

    let mut client_options = ClientOptions::new();

    // Specify client connection opthons and which broker to connect to
    let proxy_client = client_options.set_keep_alive(5)
                                    .set_reconnect(5)
                                    .connect("localhost:1883");

    // Connects to a broker and returns a `Publisher` and `Subscriber`
    let (_, subscriber) = proxy_client.start().expect("Coudn't start");
        
    let topics = vec![("hello/world", QoS::Level0), ("hello/rust", QoS::Level1)];

    subscriber.subscribe(topics, |message| {
        println!("@@@ {:?}", message);
    }).expect("Subcription failure");

    thread::sleep(Duration::new(120, 0));
}



// fn lookup_ipv4(host: &str, port: u16) -> SocketAddr {
//     use std::net::ToSocketAddrs;

//     let addrs = (host, port).to_socket_addrs().unwrap();
//     for addr in addrs {
//         if let SocketAddr::V4(_) = addr {
//             return addr.clone();
//         }
//     }

//     unreachable!("Cannot lookup address");
// }

// //#[test]
// fn tls_test() {
//     env_logger::init().unwrap();

//     let ca = "/home/ravitejareddy/Desktop/rumqtt/tests/ca.crt";
//     let ssl = SslContext::with_ca(ca).unwrap();


//     //let addr = "localhost:8883".to_socket_addrs().unwrap().next().expect("Socket address is broken");
//     let addr = lookup_ipv4("localhost", 8883);
//     let stream = TcpStream::connect(&addr).expect("Connection error");
//     ssl::SslStream::connect(&*ssl.inner, stream).expect("Ssl connection error");

//     thread::sleep(Duration::new(120, 0));
// }
