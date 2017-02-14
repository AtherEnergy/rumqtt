use std::net::{SocketAddr, ToSocketAddrs};
use std::str;
use std::sync::Arc;
use std::thread;
use std::sync::mpsc::{channel, Sender, Receiver};

use mqtt::{QualityOfService, TopicFilter};
use mqtt::control::variable_header::PacketIdentifier;
use mqtt::packet::*;
use mqtt::topic_name::TopicName;

// TODO: Refactor with quick error
use error::Result;
use message::Message;
use clientoptions::MqttOptions;
use connection::{Connection, NetworkRequest, NetworkNotification, MqttState};


pub enum MiscNwRequest {
    Disconnect,
    Shutdown,
}

pub type MessageSendableFn = Box<Fn(Message) + Send + Sync>;
pub type PublishSendableFn = Box<Fn(Message) + Send + Sync>;

/// Handles commands from Publisher and Subscriber. Saves MQTT
/// state and takes care of retransmissions.
pub struct MqttClient {
    pub opts: MqttOptions,
    pub last_pkid: PacketIdentifier,
    pub nw_request_tx: Sender<NetworkRequest>,
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

    /// Connects to the broker and starts an event loop in a new thread.
    /// Returns 'Request' and handles reqests from it.
    /// Also handles network events, reconnections and retransmissions.
    pub fn connect(opts: MqttOptions) -> Result<Self> {
        let (nw_request_tx, nw_request_rx) = channel::<NetworkRequest>();
        // let opts = opts.clone();
        let addr = Self::lookup_ipv4(opts.addr.as_str());
        let mut connection = Connection::start(addr, opts.clone(), nw_request_rx, None, None)?;
        // This thread handles network reads (coz they are blocking) and
        // and sends them to event loop thread to handle mqtt state.
        thread::spawn(move || -> Result<()> {
            let _ = connection.run();
            error!("Network Thread Stopped !!!!!!!!!");
            Ok(())
        });
        
        let client = MqttClient {
            last_pkid: PacketIdentifier(0),
            opts: opts,
            nw_request_tx: nw_request_tx,
        };
        
        Ok(client)
    }

    fn subscribe(&mut self, topics: Vec<(&str, QualityOfService)>) -> Result<()> {
         let mut sub_topics = vec![];
        for topic in topics {
            let topic = (try!(TopicFilter::new_checked(topic.0)), topic.1);
            sub_topics.push(topic);
        }

        try!(self.nw_request_tx.send(NetworkRequest::Subscribe(sub_topics)));
        Ok(())
    }

    fn publish0(&self, message: Message) -> Result<()> {
        self.nw_request_tx.send(NetworkRequest::Publish(message))?;
        Ok(())
    }

    fn publish1(&mut self, mut message: Message) -> Result<()> {
        // Add next packet id to message and publish
        let PacketIdentifier(pkid) = self._next_pkid();
        message.set_pkid(pkid);

        self.nw_request_tx.send(NetworkRequest::Publish(message))?;
        Ok(())
    }

    fn publish2(&mut self, mut message: Message) -> Result<()> {
        // Add next packet id to message and publish
        let PacketIdentifier(pkid) = self._next_pkid();
        message.set_pkid(pkid);

        try!(self.nw_request_tx.send(NetworkRequest::Publish(message)));
        Ok(())
    }

    pub fn publish(&mut self, topic: &str, qos: QualityOfService, payload: Vec<u8>) -> Result<()> {
        self._publish(topic, false, qos, payload, None)
    }

    pub fn retained_publish(&mut self, topic: &str, qos: QualityOfService, payload: Vec<u8>) -> Result<()> {
        self._publish(topic, true, qos, payload, None)
    }

    pub fn userdata_publish(&mut self, topic: &str, qos: QualityOfService, payload: Vec<u8>, userdata: Vec<u8>) -> Result<()> {
        self._publish(topic, false, qos, payload, Some(userdata))
    }

    pub fn retained_userdata_publish(&mut self,
                                     topic: &str,
                                     qos: QualityOfService,
                                     payload: Vec<u8>,
                                     userdata: Vec<u8>)
                                     -> Result<()> {
        self._publish(topic, true, qos, payload, Some(userdata))
    }

    pub fn disconnect(&self) -> Result<()> {
        try!(self.nw_request_tx.send(NetworkRequest::Disconnect));
        Ok(())
    }

    pub fn shutdown(&self) -> Result<()> {
        try!(self.nw_request_tx.send(NetworkRequest::Shutdown));
        Ok(())
    }

    fn _publish(&mut self,
                topic: &str,
                retain: bool,
                qos: QualityOfService,
                payload: Vec<u8>,
                userdata: Option<Vec<u8>>) -> Result<()> {
        let topic = try!(TopicName::new(topic.to_string()));
        let qos_pkid = match qos {
            QualityOfService::Level0 => QoSWithPacketIdentifier::Level0,
            QualityOfService::Level1 => QoSWithPacketIdentifier::Level1(0),
            QualityOfService::Level2 => QoSWithPacketIdentifier::Level2(0),
        };

        // TODO: Why are qos and pkid in the same structure
        let message = Message {
            topic: topic,
            retain: retain,
            qos: qos_pkid,
            // Optimizes clones
            payload: Arc::new(payload),
            userdata: userdata.map(|u| Arc::new(u)),
        };

        // TODO: Check message sanity here and return error if not
        match qos {
            QualityOfService::Level0 => self.publish0(message)?,
            QualityOfService::Level1 => self.publish1(message)?,
            QualityOfService::Level2 => self.publish2(message)?
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

    #[test]
    fn next_pkid_roll() {
        let client_options = MqttOptions::new();
        let mut mq_client = MqttClient::connect(client_options).unwrap();

        for i in 0..65536 {
            mq_client._next_pkid();
        }

        assert_eq!(PacketIdentifier(1), mq_client.last_pkid);
    }
}
