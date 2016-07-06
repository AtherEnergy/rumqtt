extern crate rumqtt;
extern crate mqtt;
#[macro_use]
extern crate log;
extern crate env_logger;
#[cfg(feature = "ssl")]
extern crate openssl;
extern crate mioco;

use rumqtt::{ClientOptions, ReconnectMethod, SslContext};
use mqtt::{TopicFilter, QualityOfService};
use mioco::tcp::TcpStream;
use std::thread;
use std::time::Duration;
use std::net::ToSocketAddrs;
use openssl::ssl::{self, SslMethod};
use openssl::x509::X509FileType;
use std::net::SocketAddr;


fn lookup_ipv4(host: &str, port: u16) -> SocketAddr {
    use std::net::ToSocketAddrs;

    let addrs = (host, port).to_socket_addrs().unwrap();
    for addr in addrs {
        if let SocketAddr::V4(_) = addr {
            return addr.clone();
        }
    }

    unreachable!("Cannot lookup address");
}

//#[test]
fn tls_test() {
    env_logger::init().unwrap();

    let ca = "/home/ravitejareddy/Desktop/rumqtt/tests/ca.crt";
    let ssl = SslContext::with_ca(ca).unwrap();


    //let addr = "localhost:8883".to_socket_addrs().unwrap().next().expect("Socket address is broken");
    let addr = lookup_ipv4("localhost", 8883);
    let stream = TcpStream::connect(&addr).expect("Connection error");
    ssl::SslStream::connect(&*ssl.inner, stream).expect("Ssl connection error");

    thread::sleep(Duration::new(120, 0));
}

//#[test]
fn tls_connect() {
    // USAGE: RUST_LOG=rumqtt cargo test -- --nocapture
    env_logger::init().unwrap();

    let mut client_options = ClientOptions::new();
    client_options.set_keep_alive(5);
    client_options.set_reconnect(ReconnectMethod::ReconnectAfter(Duration::new(5,0)));
    let ca = "./tests/ca.crt";
    let cert = "./tests/scooter.crt";
    let key = "./tests/scooter.key";
    let ssl = SslContext::with_ca(ca).expect("#Ssl context error");
    //let ssl = SslContext::with_cert_key_and_ca(cert, key, ca).expect("#Ssl context error");
    //let ssl = SslContext::with_cert_and_key(cert, key).expect("#Ssl context error");

    client_options.set_tls(ssl);

    let proxy_client = client_options.connect("localhost:8883").expect("CONNECT ERROR");
    match proxy_client.await() {
        Ok(_) => (),
        Err(e) => panic!("Await Error --> {:?}", e),
    };

    thread::sleep(Duration::new(120, 0));
}   

// #[test]
fn pingreq_reconnect_test() {
    // USAGE: RUST_LOG=rumqtt cargo test -- --nocapture
    env_logger::init().unwrap();

    let mut client_options = ClientOptions::new();
    client_options.set_keep_alive(5);
    client_options.set_reconnect(ReconnectMethod::ReconnectAfter(Duration::new(5,0)));
    let proxy_client = client_options.connect("localhost:1883").expect("CONNECT ERROR");
    match proxy_client.await() {
        Ok(_) => (),
        Err(e) => panic!("Await Error --> {:?}", e),
    };

    thread::sleep(Duration::new(120, 0));
}

/// Test publishes along with ping requests and responses
/// Observe if the boker is getting ping requests with in keep_alive time
/// Add handling in client if pingresp isn't received for a ping request
// #[test]
fn publish_qos0_test() {
    env_logger::init().unwrap();

    let mut client_options = ClientOptions::new();
    client_options.set_keep_alive(5);
    client_options.set_reconnect(ReconnectMethod::ReconnectAfter(Duration::new(5,0)));
    let proxy_client = client_options.connect("localhost:1883").expect("CONNECT ERROR");

    let (publisher, _) = match proxy_client.await() {
        Ok((publisher, _)) => (publisher, ..),
        Err(e) => panic!("Await Error --> {:?}", e),
    };

    for i in 0..100 {
        let payload = format!("{}. hello rust", i);
        publisher.publish("hello/rust", QualityOfService::Level0, payload.into_bytes());
        thread::sleep(Duration::new(1, 0));
    }

    thread::sleep(Duration::new(120, 0));
}

#[test]
fn publish_qos1_test() {
    env_logger::init().unwrap();

    let mut client_options = ClientOptions::new();
    client_options.set_keep_alive(5).set_pub_q_len(10);
    client_options.set_reconnect(ReconnectMethod::ReconnectAfter(Duration::new(5,0)));
    let proxy_client =
     client_options.connect("localhost:1883").expect("CONNECT ERROR");

    let (publisher, subscriber) = match proxy_client.await() {
        Ok(h) => h,
        Err(e) => panic!("Await Error --> {:?}", e),
    };

    for i in 0..100 {
        let payload = format!("{}. hello rust", i);
        publisher.publish("hello/rust", QualityOfService::Level1, payload.into_bytes());
        thread::sleep(Duration::new(1, 0));
    }

    thread::sleep(Duration::new(120, 0));
}

//#[test]
fn subscribe_test() {
    env_logger::init().unwrap();

    let mut client_options = ClientOptions::new();
    client_options.set_keep_alive(5);
    client_options.set_reconnect(ReconnectMethod::ReconnectAfter(Duration::new(5,0)));
    let proxy_client = client_options.connect("localhost:1883").expect("CONNECT ERROR");

    let (_, subscriber) = match proxy_client.await() {
        Ok((_, subcriber)) => (.., subcriber),
        Err(e) => panic!("Await Error --> {:?}", e),
    };
        
    let topics: Vec<(TopicFilter, QualityOfService)> =
    vec![(TopicFilter::new_checked("hello/world".to_string()).unwrap(),
              QualityOfService::Level0)];

    subscriber.subscribe(topics);
    thread::spawn(move || {
        loop {
            let message = subscriber.receive().unwrap();
            println!("@@@ {:?}", message);
        }
    });

    thread::sleep(Duration::new(120, 0));
}
