use std::net::{SocketAddr, ToSocketAddrs};
use std::str;
use std::sync::Arc;
use std::thread;
use std::sync::mpsc::{channel, Sender, Receiver};

use mqtt::{QualityOfService, TopicFilter};
use mqtt::control::variable_header::PacketIdentifier;

// TODO: Refactor with quick error
use error::Result;
use message::Message;
use clientoptions::MqttOptions;
use request::MqRequest;
use connection::{Connection, NetworkRequest, NetworkNotification, MqttState};


pub enum MiscNwRequest {
    Disconnect,
    Shutdown,
}

// static mut N: i32 = 0;
// unsafe {
//     N += 1;
//     println!("N: {}", N);
// }

pub type MessageSendableFn = Box<Fn(Message) + Send + Sync>;
pub type PublishSendableFn = Box<Fn(Message) + Send + Sync>;

/// Handles commands from Publisher and Subscriber. Saves MQTT
/// state and takes care of retransmissions.
pub struct MqttClient {
    pub opts: MqttOptions,
    pub last_pkid: PacketIdentifier,
    pub nw_request_tx: Option<Sender<NetworkRequest>>,
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

    pub fn new(opts: MqttOptions) -> Self {
        // TODO: Move state initialization to MqttClient constructor
        MqttClient {
            last_pkid: PacketIdentifier(0),
            opts: opts,
            nw_request_tx: None,
        }
    }

    // pub fn mock_start(mut self) -> Result<(Self, MqRequest,
    // mpsc::Receiver<NetworkRequest>)> {
    // let (pub0_tx, pub0_rx) =
    // channel::sync_channel::<Message>(self.opts.pub_q_len as usize);
    //     self.pub0_rx = Some(pub0_rx);
    // let (pub1_tx, pub1_rx) =
    // channel::sync_channel::<Message>(self.opts.pub_q_len as usize);
    //     self.pub1_rx = Some(pub1_rx);
    // let (pub2_tx, pub2_rx) =
    // channel::sync_channel::<Message>(self.opts.pub_q_len as usize);
    //     self.pub2_rx = Some(pub2_rx);

    // let (sub_tx, sub_rx) = channel::sync_channel::<Vec<(TopicFilter,
    // QualityOfService)>>(self.opts.sub_q_len as usize);
    //     self.sub_rx = Some(sub_rx);

    // let (nw_notification_tx, nw_notification_rx) =
    // channel::sync_channel::<NetworkNotification>(5);
    //     self.nw_notification_rx = Some(nw_notification_rx);

    //     let (nw_request_tx, nw_request_rx) = mpsc::channel::<NetworkRequest>();
    //     self.nw_request_tx = Some(nw_request_tx);

    //     let (misc_tx, misc_rx) = channel::sync_channel::<MiscNwRequest>(1);
    //     self.misc_rx = Some(misc_rx);

    //     // @ Create Request through which user interacts
    //     // @ These are the handles using which user interacts with rumqtt.
    //     let mq_request = MqRequest {
    //         pub0_tx: pub0_tx,
    //         pub1_tx: pub1_tx,
    //         pub2_tx: pub2_tx,
    //         subscribe_tx: sub_tx,
    //         misc_tx: misc_tx,
    //     };

    //     Ok((self, mq_request, nw_request_rx))
    // }

    /// Connects to the broker and starts an event loop in a new thread.
    /// Returns 'Request' and handles reqests from it.
    /// Also handles network events, reconnections and retransmissions.
    pub fn start(mut self) -> Result<()> {
        let (nw_request_tx, nw_request_rx) = channel::<NetworkRequest>();
        self.nw_request_tx = Some(nw_request_tx);

        let opts = self.opts.clone();

        // This thread handles network reads (coz they are blocking) and
        // and sends them to event loop thread to handle mqtt state.
        let addr = opts.addr.clone();
        let addr = Self::lookup_ipv4(addr.as_str());
        let mut connection = Connection::start(addr, opts, nw_request_rx, None, None)?;
        thread::spawn(move || -> Result<()> {
            let _ = connection.run();
            error!("Network Thread Stopped !!!!!!!!!");
            Ok(())
        });

        Ok(())
    }


    fn subscribe(&mut self, topics: Vec<(TopicFilter, QualityOfService)>) -> Result<()> {
        let nw_request_tx = self.nw_request_tx.as_ref().unwrap();
        try!(nw_request_tx.send(NetworkRequest::Subscribe(topics)));
        Ok(())
    }

    fn publish0(&mut self, message: Message) -> Result<()> {
        let nw_request_tx = self.nw_request_tx.as_ref().unwrap();
        nw_request_tx.send(NetworkRequest::Publish(message))?;
        Ok(())
    }

    fn publish1(&mut self, mut message: Message) -> Result<()> {
        // Add next packet id to message and publish
        let PacketIdentifier(pkid) = self._next_pkid();
        message.set_pkid(pkid);

        let nw_request_tx = self.nw_request_tx.as_ref().unwrap();
        nw_request_tx.send(NetworkRequest::Publish(message))?;
        Ok(())
    }

    fn publish2(&mut self, mut message: Message) -> Result<()> {
        // Add next packet id to message and publish
        let PacketIdentifier(pkid) = self._next_pkid();
        message.set_pkid(pkid);

        let nw_request_tx = self.nw_request_tx.as_ref().unwrap();
        try!(nw_request_tx.send(NetworkRequest::Publish(message)));
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
        let mut mq_client = MqttClient::new(client_options);

        for i in 0..65536 {
            mq_client._next_pkid();
        }

        assert_eq!(PacketIdentifier(1), mq_client.last_pkid);
    }
}
