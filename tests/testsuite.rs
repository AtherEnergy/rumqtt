extern crate cloudpubsub;

use cloudpubsub::{MqttOptions, MqttClient, MqttCallback, Message};

#[macro_use]
extern crate log;
extern crate pretty_env_logger;

use std::thread;
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const BROKER_ADDRESS: &'static str = "dev-mqtt-broker.atherengineering.in:1883";
const MOSQUITTO_ADDR: &'static str = "test.mosquitto.org:1883";
const MOSQUITTO_LOCAL: &'static str = "localhost:1883";

#[test]
fn acked_message() {
    let client_options = MqttOptions::new()
        .set_reconnect_interval(5)
        .set_client_id("test-reconnect-client")
        .set_broker(BROKER_ADDRESS);

    let cb = |m: Message| {
        let ref payload = *m.payload;
        let ref userdata = *m.userdata.unwrap();
        let payload = String::from_utf8(payload.clone()).unwrap();
        let userdata = String::from_utf8(userdata.clone()).unwrap();
        assert_eq!("MYUNIQUEMESSAGE".to_string(), payload);
        assert_eq!("MYUNIQUEUSERDATA".to_string(), userdata);
    };

    let msg_callback = MqttCallback::new().on_message(cb);

    // Connects to a broker and returns a `request`
    let mut request = MqttClient::start(client_options, Some(msg_callback)).expect("Couldn't start");
    request.userdata_publish("test/qos1/ack",
                          "MYUNIQUEMESSAGE".to_string().into_bytes(),
                          "MYUNIQUEUSERDATA".to_string().into_bytes())
        .unwrap();
    thread::sleep(Duration::new(1, 0));
}

#[test]
fn simple_stress_publish() {
    // pretty_env_logger::init().unwrap();
    let client_options = MqttOptions::new()
        .set_reconnect_interval(3)
        .set_client_id("qos1-stress-publish")
        .set_broker(BROKER_ADDRESS);

    // TODO: Alert!!! Mosquitto seems to be unable to publish fast (loosing
    // messsages with mosquitto broker. local and remote)

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let cb = move |_: Message| {
        // println!("ack: {:?}", m.pkid);
        count.fetch_add(1, Ordering::SeqCst);
    };

    let callback = MqttCallback::new().on_publish(cb);

    let mut client = MqttClient::start(client_options, Some(callback)).expect("Coudn't start");

    for i in 0..1010 {
        let payload = format!("{}. hello rust", i);
        client.publish("test/qos1/stress", payload.clone().into_bytes()).unwrap();
    }

    thread::sleep(Duration::new(15, 0));
    println!("QoS1 Final Count = {:?}", final_count.load(Ordering::SeqCst));
    assert!(1010 <= final_count.load(Ordering::SeqCst));
}

#[test]
fn stress_publish_with_reconnections() {
    // pretty_env_logger::init().unwrap();

    let client_options = MqttOptions::new()
        .set_reconnect_interval(3)
        .set_client_id("qos1-stress-reconnect-publish")
        .set_clean_session(false)
        .set_broker(BROKER_ADDRESS);

    let count = Arc::new(AtomicUsize::new(0));
    let final_count = count.clone();
    let count = count.clone();

    let cb = move |_: Message| {
        // println!("ack: {:?}", m.pkid);
        count.fetch_add(1, Ordering::SeqCst);
    };

    let callback = MqttCallback::new().on_publish(cb);

    let mut client = MqttClient::start(client_options, Some(callback)).expect("Coudn't start");

    for i in 0..1010 {
        let payload = format!("{}. hello rust", i);
        if i == 250 || i == 500 || i == 750 {
            let _ = client.disconnect();
        }
        client.publish("test/qos1/reconnection_stress", payload.clone().into_bytes()).unwrap();
    }

    thread::sleep(Duration::new(15, 0));
    println!("QoS1 Final Count = {:?}", final_count.load(Ordering::SeqCst));
    assert!(1010 <= final_count.load(Ordering::SeqCst));
}