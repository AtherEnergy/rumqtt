use std::net::{SocketAddr, ToSocketAddrs};
use std::str;
use std::sync::Arc;
use std::thread;
use std::sync::mpsc::{sync_channel, SyncSender};

use mqtt::{QualityOfService, TopicFilter};
use mqtt::control::variable_header::PacketIdentifier;
use mqtt::packet::*;
use mqtt::topic_name::TopicName;

// TODO: Refactor with quick error
use error::Result;
use message::Message;
use clientoptions::MqttOptions;
use connection::{Connection, NetworkRequest};


pub type MessageSendableFn = Box<Fn(Message) + Send + Sync>;
pub type PublishSendableFn = Box<Fn(Message) + Send + Sync>;

/// Handles commands from Publisher and Subscriber. Saves MQTT
/// state and takes care of retransmissions.
pub struct MqttClient {
    pub last_pkid: PacketIdentifier,
    pub nw_request_tx: SyncSender<NetworkRequest>,
}

impl MqttClient {
    fn lookup_ipv4<A: ToSocketAddrs>(addr: A) -> SocketAddr {
        let addrs = addr.to_socket_addrs().expect("Conversion Failed");
        for addr in addrs {
            if let SocketAddr::V4(_) = addr {
                return addr;
            }
        }
        unreachable!("Cannot lookup address");
    }

    fn mock_start(opts: MqttOptions) -> Result<Self> {
        let (nw_request_tx, nw_request_rx) = sync_channel::<NetworkRequest>(50);

        thread::spawn(move || -> Result<()> {
            let _ = nw_request_rx;
            thread::sleep_ms(1000_000);
            Ok(())
        });

        let client = MqttClient {
            last_pkid: PacketIdentifier(0),
            nw_request_tx: nw_request_tx,
        };

        Ok(client)
    }

    /// Connects to the broker and starts an event loop in a new thread.
    /// Returns 'Request' and handles reqests from it.
    /// Also handles network events, reconnections and retransmissions.
    pub fn start(opts: MqttOptions) -> Result<Self> {
        let (nw_request_tx, nw_request_rx) = sync_channel::<NetworkRequest>(50);
        let addr = Self::lookup_ipv4(opts.addr.as_str());
        let mut connection = Connection::connect(addr, opts.clone(), nw_request_rx, None, None)?;
        // This thread handles network reads (coz they are blocking) and
        // and sends them to event loop thread to handle mqtt state.
        thread::spawn(move || -> Result<()> {
            let _ = connection.run();
            error!("Network Thread Stopped !!!!!!!!!");
            Ok(())
        });

        let client = MqttClient {
            last_pkid: PacketIdentifier(0),
            nw_request_tx: nw_request_tx,
        };

        Ok(client)
    }

    pub fn subscribe(&mut self, topics: Vec<(&str, QualityOfService)>) -> Result<()> {
        let mut sub_topics = Vec::with_capacity(topics.len());
        for topic in topics {
            let topic = (TopicFilter::new_checked(topic.0)?, topic.1);
            sub_topics.push(topic);
        }
        self.nw_request_tx.send(NetworkRequest::Subscribe(sub_topics))?;
        Ok(())
    }


    pub fn publish(&mut self, topic: &str, qos: QualityOfService, payload: Vec<u8>) -> Result<()> {
        let payload = Arc::new(payload);
        loop {
            let payload = payload.clone();
            let _ = self._publish(topic, false, qos, payload, None);
            warn!("Request Queue Full !!!!!!!!");
            thread::sleep_ms(2000);
        }
    }

    pub fn retained_publish(&mut self, topic: &str, qos: QualityOfService, payload: Vec<u8>) -> Result<()> {
        let payload = Arc::new(payload);
        self._publish(topic, true, qos, payload, None)
    }

    pub fn userdata_publish(&mut self, topic: &str, qos: QualityOfService, payload: Vec<u8>, userdata: Vec<u8>) -> Result<()> {
        let payload = Arc::new(payload);
        self._publish(topic, false, qos, payload, Some(userdata))
    }

    pub fn retained_userdata_publish(&mut self,
                                     topic: &str,
                                     qos: QualityOfService,
                                     payload: Vec<u8>,
                                     userdata: Vec<u8>)
                                     -> Result<()> {
        let payload = Arc::new(payload);
        self._publish(topic, true, qos, payload, Some(userdata))
    }

    pub fn disconnect(&self) -> Result<()> {
        self.nw_request_tx.send(NetworkRequest::Disconnect)?;
        Ok(())
    }

    pub fn shutdown(&self) -> Result<()> {
        self.nw_request_tx.send(NetworkRequest::Shutdown)?;
        Ok(())
    }

    fn _publish(&mut self,
                topic: &str,
                retain: bool,
                qos: QualityOfService,
                payload: Arc<Vec<u8>>,
                userdata: Option<Vec<u8>>)
                -> Result<()> {

        let topic = TopicName::new(topic.to_string())?;
        let qos_pkid = match qos {
            QualityOfService::Level0 => QoSWithPacketIdentifier::Level0,
            QualityOfService::Level1 => QoSWithPacketIdentifier::Level1(0),
            QualityOfService::Level2 => QoSWithPacketIdentifier::Level2(0),
        };

        let mut message = Message {
            topic: topic,
            retain: retain,
            qos: qos_pkid,
            payload: payload,
            userdata: userdata.map(Arc::new),
        };

        // TODO: Check message sanity here and return error if not
        match qos {
            QualityOfService::Level0 => self.nw_request_tx.send(NetworkRequest::Publish(message))?,
            QualityOfService::Level1 |
            QualityOfService::Level2 => {
                let PacketIdentifier(pkid) = self._next_pkid();
                message.set_pkid(pkid);
                self.nw_request_tx.try_send(NetworkRequest::Publish(message))?
            }
        };

        Ok(())
    }

    // http://stackoverflow.
    // com/questions/11115364/mqtt-messageid-practical-implementation
    #[inline]
    fn _next_pkid(&mut self) -> PacketIdentifier {
        let PacketIdentifier(mut pkid) = self.last_pkid;
        if pkid == 65535 {
            pkid = 0;
        }
        self.last_pkid = PacketIdentifier(pkid + 1);
        self.last_pkid
    }
}

// @@@@@@@@@@@@~~~~~~~UNIT TESTS ~~~~~~~~~@@@@@@@@@@@@

#[cfg(test)]
mod test {
    #![allow(unused_variables)]
    extern crate env_logger;
    use mqtt::QualityOfService as QoS;
    use connection::MqttState;
    use clientoptions::MqttOptions;
    use mqtt::control::variable_header::PacketIdentifier;
    use super::MqttClient;
    use std::sync::Arc;

    #[test]
    fn next_pkid_roll() {
        let client_options = MqttOptions::new().broker("test.mosquitto.org:1883");
        match MqttClient::mock_start(client_options) {
            Ok(mut mq_client) => {
                for i in 0..65536 {
                    mq_client._next_pkid();
                }
                assert_eq!(PacketIdentifier(1), mq_client.last_pkid);
            }
            Err(e) => panic!("{:?}", e),
        }

    }

    #[test]
    #[should_panic]
    fn request_queue_blocks_when_buffer_full() {
        env_logger::init().unwrap();
        let client_options = MqttOptions::new().broker("test.mosquitto.org:1883");
        match MqttClient::mock_start(client_options) {
            Ok(mut mq_client) => {
                for i in 0..65536 {
                    mq_client._publish("hello/world", false, QoS::Level1, Arc::new(vec![1u8, 2, 3]), None).unwrap();
                }
            }
            Err(e) => panic!("{:?}", e),
        }
    }
}
