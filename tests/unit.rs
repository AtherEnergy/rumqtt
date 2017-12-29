
#[cfg(test)]
mod tests {
    extern crate rumqtt;
    extern crate loggerv;

    use std::thread;
    use std::time::Duration;
    use std::sync::{Arc, Mutex};

    use self::rumqtt::{MqttOptions, ReconnectOptions, MqttClient, QoS};

    #[test]
    fn basic_publish_notifications() {
        loggerv::init_with_verbosity(1).unwrap();
        let mqtt_opts = MqttOptions::new("rumqtt-core", "127.0.0.1:1883").unwrap()
                                    .set_reconnect_opts(ReconnectOptions::Always(10));

        let (mut client, receiver) = MqttClient::start(mqtt_opts);
        let counter = Arc::new(Mutex::new(0));

        client.subscribe(vec![("hello/world", QoS::AtLeastOnce)]);

        let counter_clone = counter.clone();
        thread::spawn(move || {
            for i in receiver {
                *counter_clone.lock().unwrap() += 1;
            }
        });

        for i in 0..3 {
            if let Err(e) = client.publish("hello/world", QoS::AtLeastOnce, vec![1, 2, 3]) {
                println!("{:?}", e);
            }
            thread::sleep(Duration::new(1, 0));
        }

        thread::sleep(Duration::new(10, 0));
        // 1 for suback
        // 3 for puback
        // 3 for actual published messages
        assert_eq!(*counter.lock().unwrap(), 7);
    }

    #[test]
    #[should_panic]
    fn client_id_startswith_space() {
        let mqtt_opts = MqttOptions::new(" client_a", "127.0.0.1:1883").unwrap()
                                    .set_reconnect_opts(ReconnectOptions::Always(10))
                                    .set_clean_session(true);
    }

    #[test]
    #[should_panic]
    fn no_client_id() {
        let mqtt_opts = MqttOptions::new("", "127.0.0.1:1883").unwrap()
                                    .set_reconnect_opts(ReconnectOptions::Always(10))
                                    .set_clean_session(true);
    }

    #[test]
    fn retain_msg_when_client_goes_offline() {
        // TODO This goes into infinite loop when two mqtt clients are started with
        // the same client id
        
        {
            let mqtt_opts = MqttOptions::new("offline", "127.0.0.1:1883").unwrap()
                                    .set_reconnect_opts(ReconnectOptions::Always(10))
                                    .set_clean_session(false);
            let (mut client_a, receiver_a) = MqttClient::start(mqtt_opts);
            // client_a.subscribe(vec![("hello/world", QoS::AtLeastOnce)]);
            // client_a.disconnect();
            thread::sleep_ms(5000);
        }

        // {

        //     let mqtt_opts = MqttOptions::new("second", "127.0.0.1:1883").unwrap()
        //                                 .set_reconnect_opts(ReconnectOptions::Always(10))
        //                                 .set_clean_session(true);
        //     let (mut client_b, receiver_b) = MqttClient::start(mqtt_opts);
        //     if let Err(e) = client_b.publish("hello/world", QoS::AtLeastOnce, vec![1, 2, 3]) {
        //         println!("Error publishing from client b {:?}", e);
        //     }

        //     client_b.disconnect();
        //     thread::sleep_ms(5000);

        // }

        let mqtt_opts = MqttOptions::new("offline", "127.0.0.1:1883").unwrap()
                                    // .set_reconnect_opts(ReconnectOptions::Always(10))
                                    .set_clean_session(false);
        let (mut client_b, receiver_b) = MqttClient::start(mqtt_opts);

        // for i in receiver_a {
        //     println!("From offline session {:?}", i);
        // }

        thread::sleep_ms(10000);
    }
}