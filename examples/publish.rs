extern crate rumqtt;
extern crate pretty_env_logger;
extern crate rand;

use std::thread;
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rand::{thread_rng, Rng};

use rumqtt::{MqttOptions, MqttClient, MqttCallback, Message};

fn main() {
    pretty_env_logger::init().unwrap();

    let options = MqttOptions::new().set_client_id("publisher-1")
                                    .set_clean_session(false)
                                    .set_first_reconnect_loop(true)
                                    .set_reconnect_interval(10)
                                    //.set_broker("dev-mqtt-broker.atherengineering.in:1883");
                                    .set_broker("localhost:9883");

    let count = Arc::new(AtomicUsize::new(0));
    let callback_count = count.clone();

    let counter_cb = move |m: Message| {
        callback_count.fetch_add(1, Ordering::SeqCst);
        let ref userdata = *m.userdata.unwrap();
        let userdata = String::from_utf8(userdata.clone()).unwrap();
        assert_eq!("MYUNIQUEUSERDATA".to_string(), userdata);
    };
    let on_publish = MqttCallback::new().on_publish(counter_cb);

    let mut client = MqttClient::start(options, Some(on_publish)).expect("Start Error");

    for _ in 0..10000 {
        let len: usize = thread_rng().gen_range(0, 100_000);
        let mut v = vec![0; len];
        thread_rng().fill_bytes(&mut v);

        let _ = client.userdata_publish("hello/world", v, "MYUNIQUEUSERDATA".to_string().into_bytes());
        thread::sleep_ms(100);
    }

    // verifies pingreqs and responses
    thread::sleep(Duration::from_secs(30));

    // disconnections because of pingreq delays will be know during
    // subsequent publishes
    for _ in 0..10000 {
        let len: usize = thread_rng().gen_range(0, 100_000);
        let mut v = vec![0; len];
        thread_rng().fill_bytes(&mut v);

        let _ = client.userdata_publish("hello/world", v, "MYUNIQUEUSERDATA".to_string().into_bytes());
    }

    thread::sleep(Duration::from_secs(31));
    println!("Total Ack Count = {:?}", count);
}
