
#[cfg(test)]
mod tests {
    extern crate rumqtt;
    extern crate loggerv;

    use std::thread;
    use std::time::Duration;
    use std::sync::{Arc, Mutex};

    use self::rumqtt::{MqttOptions, ReconnectOptions, MqttClient, QoS, Packet, LastWill};

    #[test]
    fn basic_qos0_publish() {
        let mqtt_opts = MqttOptions::new("qos0publish", "127.0.0.1:1883").unwrap()
                                    .set_reconnect_opts(ReconnectOptions::AfterFirstSuccess(10));

        let (mut client, receiver) = MqttClient::start(mqtt_opts);
        client.subscribe(vec![("hello/qos0", QoS::AtLeastOnce)]).unwrap();

        let counter = Arc::new(Mutex::new(0));
        let receiver_counter = counter.clone();
        let total_count = 100;
        
        // incoming packets
        thread::spawn(move || {
            for (packet, _) in receiver {
                match packet {
                    Packet::Publish(_) => *receiver_counter.lock().unwrap() += 1,
                    _ => (),
                }    
            }
        });

        for _ in 0..total_count {
            client.publish("hello/qos0", QoS::AtMostOnce, vec![1, 2, 3]).unwrap();
            thread::sleep(Duration::from_millis(100));
        }

        thread::sleep(Duration::new(10, 0));
        assert_eq!(*counter.lock().unwrap(), total_count);
    }

    #[test]
    fn basic_qos1_publish() {
        let mqtt_opts = MqttOptions::new("qos1publish", "127.0.0.1:1883").unwrap()
                                    .set_reconnect_opts(ReconnectOptions::AfterFirstSuccess(10));

        let (mut client, receiver) = MqttClient::start(mqtt_opts);
        client.subscribe(vec![("hello/qos1", QoS::AtLeastOnce)]).unwrap();

        let counter = Arc::new(Mutex::new(0));
        let receiver_counter = counter.clone();
        let total_count = 100;
        
        // incoming packets
        thread::spawn(move || {
            for (packet, _) in receiver {
                match packet {
                    Packet::Publish(_) => *receiver_counter.lock().unwrap() += 1,
                    _ => (),
                }    
            }
        });

        for _ in 0..total_count {
            client.publish("hello/qos1", QoS::AtLeastOnce, vec![1, 2, 3]).unwrap();
            thread::sleep(Duration::from_millis(100));
        }

        thread::sleep(Duration::new(10, 0));
        assert_eq!(*counter.lock().unwrap(), total_count);
    }

    #[test]
    fn userdata_qos1_publish() {
        let mqtt_opts = MqttOptions::new("qos1publish", "127.0.0.1:1883").unwrap()
                                    .set_reconnect_opts(ReconnectOptions::AfterFirstSuccess(10));

        let (mut client, receiver) = MqttClient::start(mqtt_opts);
        client.subscribe(vec![("hello/qos1", QoS::AtLeastOnce)]).unwrap();

        let counter = Arc::new(Mutex::new(0));
        let receiver_counter = counter.clone();
        let total_count = 100;
        
        // incoming packets
        thread::spawn(move || {
            for (packet, userdata) in receiver {
                match packet {
                    Packet::Publish(_) => *receiver_counter.lock().unwrap() += 1,
                    Packet::Puback(_) => assert_eq!(userdata.unwrap(), "hello"),
                    _ => (),
                }    
            }
        });

        for _ in 0..total_count {
            client.publish_with_userdata("hello/qos1", QoS::AtLeastOnce, vec![1, 2, 3], "hello").unwrap();
            thread::sleep(Duration::from_millis(100));
        }

        thread::sleep(Duration::new(10, 0));
        assert_eq!(*counter.lock().unwrap(), total_count);
    }

    #[test]
    fn connect_with_will() {
        let mqtt_opts = MqttOptions::new("willconnect", "127.0.0.1:1883").unwrap();
        let (mut client1, receiver1) = MqttClient::start(mqtt_opts);
        client1.subscribe(vec![("iam/dead", QoS::AtLeastOnce)]).unwrap();

        {
            let will = LastWill{topic: "iam/dead".to_owned(), message: "dead".to_string(), qos: QoS::AtLeastOnce, retain: false};
            let mqtt_opts = MqttOptions::new("willpublish", "127.0.0.1:1883").unwrap()
                                    .set_reconnect_opts(ReconnectOptions::AfterFirstSuccess(10))
                                    .set_last_will(will);

            let (_client, _receiver) = MqttClient::start(mqtt_opts);
            thread::sleep(Duration::new(2, 0));
        }

        for (message, _) in receiver1 {
            match message {
                Packet::Publish(publish) => {
                    if publish.topic_name != "iam/dead" {
                        panic!("Didn't receive will message");
                    } else {
                        break
                    }
                }
                _ => continue,
            }
        }
    }
}