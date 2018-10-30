use std::collections::VecDeque;
use std::result::Result;
use std::time::Instant;

use client::{Notification, Request};
use error::{ConnectError, NetworkError};
use mqtt3::{
    Connack, Connect, ConnectReturnCode, Packet, PacketIdentifier, Publish, QoS, Subscribe,
};
use mqttoptions::{MqttOptions, SecurityOptions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MqttConnectionStatus {
    Handshake,
    Connected,
    Disconnected,
}

#[derive(Debug)]
pub(crate) struct MqttState {
    pub opts: MqttOptions,

    // --------  State  ----------
    connection_status: MqttConnectionStatus,
    await_pingresp: bool,
    last_network_activity: Instant,
    last_pkid: PacketIdentifier,

    // Stores outgoing data to handle quality of service
    outgoing_pub: VecDeque<Publish>, // QoS1 & 2 publishes
    outgoing_rel: VecDeque<PacketIdentifier>,

    // Store incoming data to handle quality of service
    incoming_pub: VecDeque<PacketIdentifier>, // QoS2 publishes
}

/// Design: `MqttState` methods will just modify the state of the object
///         but doesn't do any network operations. Methods will do
///         appropriate returns so that n/w methods or n/w eventloop can
///         operate directly. This abstracts the functionality better
///         so that it's easy to switch between synchronous code, tokio (or)
///         async/await

impl MqttState {
    pub fn new(opts: MqttOptions) -> Self {
        MqttState { opts,
                    connection_status: MqttConnectionStatus::Disconnected,
                    await_pingresp: false,
                    last_network_activity: Instant::now(),
                    last_pkid: PacketIdentifier(0),
                    outgoing_pub: VecDeque::new(),
                    outgoing_rel: VecDeque::new(),
                    incoming_pub: VecDeque::new() }
    }

    pub fn handle_outgoing_mqtt_packet(&mut self, packet: Packet) -> Result<Packet, NetworkError> {
        match packet {
            Packet::Publish(publish) => {
                let publish = self.handle_outgoing_publish(publish)?;
                Ok(Packet::Publish(publish))
            }
            Packet::Pingreq => {
                let _ping = self.handle_outgoing_ping()?;
                Ok(Packet::Pingreq)
            }
            Packet::Subscribe(subs) => {
                let subscription = self.handle_outgoing_subscribe(subs)?;
                Ok(Packet::Subscribe(subscription))
            }
            _ => Ok(packet),
        }
    }

    // Takes incoming mqtt packet, applies state changes and returns notifiaction packet and
    // network reply packet.
    // Notification packet should be sent to the user and Mqtt reply packet which should be sent
    // back on network
    //
    // E.g For incoming QoS1 publish packet, this method returns (Publish, Puback). Publish packet will
    // be forwarded to user and Pubck packet will be written to network
    pub fn handle_incoming_mqtt_packet(&mut self,
                                       packet: Packet)
                                       -> Result<(Notification, Request), NetworkError> {
        self.update_last_in_control_time();

        match packet {
            Packet::Pingresp => self.handle_incoming_pingresp(),
            Packet::Publish(publish) => self.handle_incoming_publish(publish.clone()),
            Packet::Suback(_pkid) => Ok((Notification::None, Request::None)),
            Packet::Puback(pkid) => self.handle_incoming_puback(pkid),
            Packet::Pubrec(pkid) => self.handle_incoming_pubrec(pkid),
            Packet::Pubrel(pkid) => self.handle_incoming_pubrel(pkid),
            Packet::Pubcomp(pkid) => self.handle_incoming_pubcomp(pkid),
            _ => panic!("{:?}", packet),
        }
    }

    pub fn handle_outgoing_connect(&mut self) -> Result<Connect, ConnectError> {
        self.connection_status = MqttConnectionStatus::Handshake;

        let (_username, _password) = match self.opts.security {
            SecurityOptions::UsernamePassword((ref username, ref password)) => {
                (Some(username.to_owned()), Some(password.to_owned()))
            }
            _ => (None, None),
        };

        self.opts.connect_packet()
    }

    pub fn handle_incoming_connack(&mut self, connack: Connack) -> Result<(), ConnectError> {
        let response = connack.code;
        if response != ConnectReturnCode::Accepted {
            self.connection_status = MqttConnectionStatus::Disconnected;
            Err(ConnectError::MqttConnectionRefused(response.to_u8()))
        } else {
            self.connection_status = MqttConnectionStatus::Connected;
            self.handle_previous_session();

            Ok(())
        }
    }

    pub fn handle_reconnection(&mut self) -> VecDeque<Packet> {
        if self.opts.clean_session {
            VecDeque::new()
        } else {
            //TODO: Write unittest for checking state during reconnection
            self.outgoing_pub
                .clone()
                .into_iter()
                .map(|publish| Packet::Publish(publish))
                .collect()
        }
    }

    fn add_packet_id_and_save(&mut self, mut publish: Publish) -> Publish {
        let publish = if publish.pid == None {
            let pkid = self.next_pkid();
            publish.pid = Some(pkid);
            publish
        } else {
            publish
        };

        self.outgoing_pub.push_back(publish.clone());
        publish
    }

    /// Sets next packet id if pkid is None (fresh publish) and adds it to the
    /// outgoing publish queue
    pub fn handle_outgoing_publish(&mut self, publish: Publish) -> Result<Publish, NetworkError> {
        if publish.payload.len() > self.opts.max_packet_size {
            return Err(NetworkError::PacketSizeLimitExceeded);
        }

        let publish = match publish.qos {
            QoS::AtMostOnce => publish,
            QoS::AtLeastOnce | QoS::ExactlyOnce => self.add_packet_id_and_save(publish),
        };

        Ok(publish)
    }

    pub fn handle_incoming_puback(&mut self,
                                  pkid: PacketIdentifier)
                                  -> Result<(Notification, Request), NetworkError> {
        match self.outgoing_pub.iter().position(|x| x.pid == Some(pkid)) {
            Some(index) => {
                let _publish = self.outgoing_pub.remove(index).expect("Wrong index");
                Ok((Notification::None, Request::None))
            }
            None => {
                error!("Unsolicited puback packet: {:?}", pkid);
                Err(NetworkError::Unsolicited)
            }
        }
    }

    pub fn handle_incoming_pubrec(&mut self,
                                  pkid: PacketIdentifier)
                                  -> Result<(Notification, Request), NetworkError> {
        match self.outgoing_pub.iter().position(|x| x.pid == Some(pkid)) {
            Some(index) => {
                let _publish = self.outgoing_pub.remove(index).expect("Wrong index");
                self.outgoing_rel.push_back(pkid);

                let notification = Notification::None;
                let reply = Request::PubRel(pkid);
                Ok((notification, reply))
            }
            None => {
                error!("Unsolicited pubrec packet: {:?}", pkid);
                Err(NetworkError::Unsolicited)
            }
        }
    }

    // return a tuple. tuple.0 is supposed to be send to user through 'notify_tx' while tuple.1
    // should be sent back on network as ack
    pub fn handle_incoming_publish(&mut self,
                                   publish: Publish)
                                   -> Result<(Notification, Request), NetworkError> {
        let qos = publish.qos;

        match qos {
            QoS::AtMostOnce => Ok((Notification::None, Request::None)),
            QoS::AtLeastOnce => {
                let pkid = publish.pid.unwrap();
                let request = Request::PubAck(pkid);
                let notification = Notification::Publish(publish);
                Ok((notification, request))
            }
            QoS::ExactlyOnce => {
                let pkid = publish.pid.unwrap();
                let request = Request::PubRec(pkid);
                let notification = Notification::Publish(publish);

                self.incoming_pub.push_back(pkid);
                Ok((notification, request))
            }
        }
    }

    pub fn handle_incoming_pubrel(&mut self,
                                  pkid: PacketIdentifier)
                                  -> Result<(Notification, Request), NetworkError> {
        match self.incoming_pub.iter().position(|x| *x == pkid) {
            Some(index) => {
                let _pkid = self.incoming_pub.remove(index);
                let notification = Notification::None;
                let reply = Request::PubComp(pkid);
                Ok((notification, reply))
            }
            None => {
                error!("Unsolicited pubrel packet: {:?}", pkid);
                Err(NetworkError::Unsolicited)
            }
        }
    }

    pub fn handle_incoming_pubcomp(&mut self,
                                   pkid: PacketIdentifier)
                                   -> Result<(Notification, Request), NetworkError> {
        match self.outgoing_rel.iter().position(|x| *x == pkid) {
            Some(index) => {
                self.outgoing_rel.remove(index).expect("Wrong index");
                Ok((Notification::None, Request::None))
            }
            _ => {
                error!("Unsolicited pubcomp packet: {:?}", pkid);
                Err(NetworkError::Unsolicited)
            }
        }
    }

    // reset the last control packet sent time
    pub fn update_last_in_control_time(&mut self) {
        self.last_network_activity = Instant::now();
    }

    // check if pinging is required based on last flush time
    pub fn is_ping_required(&self) -> bool {
        let in_elapsed = self.last_network_activity.elapsed();

        debug!("Last incoming packet (network activity) before {:?} seconds. Keep alive = {:?}",
               in_elapsed.as_secs(),
               self.opts.keep_alive);
        in_elapsed >= self.opts.keep_alive
    }

    // check when the last control packet/pingreq packet
    // is received and return the status which tells if
    // keep alive time has exceeded
    // NOTE: status will be checked for zero keepalive times also
    pub fn handle_outgoing_ping(&mut self) -> Result<(), NetworkError> {
        // @ Prevents half open connections. Tcp writes will buffer up
        // with out throwing any error (till a timeout) when internet
        // is down. Even though broker closes the socket after timeout,
        // EOF will be known only after reconnection.
        // We need to unbind the socket if there in no pingresp before next ping
        // (What about case when pings aren't sent because of constant publishes
        // ?. A. Tcp write buffer gets filled up and write will be blocked for 10
        // secs and then error out because of timeout.)

        // raise error if last ping didn't receive ack
        if self.await_pingresp {
            error!("Error awaiting for last ping response");
            return Err(NetworkError::AwaitPingResp);
        }

        if self.connection_status == MqttConnectionStatus::Connected {
            self.await_pingresp = true;
            Ok(())
        } else {
            error!("State = {:?}. Shouldn't ping in this state",
                   self.connection_status);
            Err(NetworkError::InvalidState)
        }
    }

    pub fn handle_incoming_pingresp(&mut self) -> Result<(Notification, Request), NetworkError> {
        self.await_pingresp = false;
        Ok((Notification::None, Request::None))
    }

    pub fn handle_outgoing_subscribe(&mut self,
                                     mut subscription: Subscribe)
                                     -> Result<Subscribe, NetworkError> {
        let pkid = self.next_pkid();

        if self.connection_status == MqttConnectionStatus::Connected {
            subscription.pid = pkid;

            Ok(subscription)
        } else {
            error!("State = {:?}. Shouldn't subscribe in this state",
                   self.connection_status);
            Err(NetworkError::InvalidState)
        }
    }

    // pub fn handle_incoming_suback(&mut self, ack: Suback) -> Result<(), SubackError> {
    //     if ack.return_codes.iter().any(|v| *v == SubscribeReturnCodes::Failure) {
    //         Err(SubackError::Rejected)
    //     } else {
    //         Ok(())
    //     }
    // }

    fn handle_previous_session(&mut self) {
        self.await_pingresp = false;

        if self.opts.clean_session {
            self.outgoing_pub.clear();
        }

        self.last_network_activity = Instant::now();
    }

    // http://stackoverflow.com/questions/11115364/mqtt-messageid-practical-implementation
    fn next_pkid(&mut self) -> PacketIdentifier {
        let PacketIdentifier(mut pkid) = self.last_pkid;
        if pkid == 65_535 {
            pkid = 0;
        }
        self.last_pkid = PacketIdentifier(pkid + 1);
        self.last_pkid
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use super::{MqttConnectionStatus, MqttState};
    use client::Notification;
    use client::Request;
    use error::NetworkError;
    use mqtt3::*;
    use mqttoptions::MqttOptions;

    fn build_outgoing_publish(qos: QoS) -> Publish {
        Publish { dup: false,
                  qos,
                  retain: false,
                  pid: None,
                  topic_name: "hello/world".to_owned(),
                  payload: Arc::new(vec![1, 2, 3]) }
    }

    fn build_incoming_publish(qos: QoS, pkid: u16) -> Publish {
        Publish { dup: false,
                  qos,
                  retain: false,
                  pid: Some(PacketIdentifier(pkid)),
                  topic_name: "hello/world".to_owned(),
                  payload: Arc::new(vec![1, 2, 3]) }
    }

    fn build_mqttstate() -> MqttState {
        let opts = MqttOptions::new("test-id", "127.0.0.1", 1883);
        MqttState::new(opts)
    }

    #[test]
    fn next_pkid_roll() {
        let mut mqtt = build_mqttstate();
        let mut pkt_id = PacketIdentifier(0);

        for _ in 0..65536 {
            pkt_id = mqtt.next_pkid();
        }
        assert_eq!(PacketIdentifier(1), pkt_id);
    }

    #[test]
    fn outgoing_publish_handle_should_set_pkid_correctly_and_add_publish_to_queue_correctly() {
        let mut mqtt = build_mqttstate();

        // QoS0 Publish
        let publish = build_outgoing_publish(QoS::AtMostOnce);

        // Packet id shouldn't be set and publish shouldn't be saved in queue
        let publish_out = mqtt.handle_outgoing_publish(publish);
        assert_eq!(publish_out.unwrap().pid, None);
        assert_eq!(mqtt.outgoing_pub.len(), 0);

        // QoS1 Publish
        let publish = build_outgoing_publish(QoS::AtLeastOnce);

        // Packet id should be set and publish should be saved in queue
        let publish_out = mqtt.handle_outgoing_publish(publish.clone());
        assert_eq!(publish_out.unwrap().pid, Some(PacketIdentifier(1)));
        assert_eq!(mqtt.outgoing_pub.len(), 1);

        // Packet id should be incremented and publish should be saved in queue
        let publish_out = mqtt.handle_outgoing_publish(publish.clone());
        assert_eq!(publish_out.unwrap().pid, Some(PacketIdentifier(2)));
        assert_eq!(mqtt.outgoing_pub.len(), 2);

        // QoS1 Publish
        let publish = build_outgoing_publish(QoS::ExactlyOnce);

        // Packet id should be set and publish should be saved in queue
        let publish_out = mqtt.handle_outgoing_publish(publish.clone());
        assert_eq!(publish_out.unwrap().pid, Some(PacketIdentifier(3)));
        assert_eq!(mqtt.outgoing_pub.len(), 3);

        // Packet id should be incremented and publish should be saved in queue
        let publish_out = mqtt.handle_outgoing_publish(publish.clone());
        assert_eq!(publish_out.unwrap().pid, Some(PacketIdentifier(4)));
        assert_eq!(mqtt.outgoing_pub.len(), 4);
    }

    #[test]
    fn outgoing_publish_handle_should_throw_error_when_packetsize_exceeds_max() {
        let opts = MqttOptions::new("test-id", "127.0.0.1", 1883);
        let mut mqtt = MqttState::new(opts);

        let publish = Publish { dup: false,
                                qos: QoS::AtMostOnce,
                                retain: false,
                                pid: None,
                                topic_name: "hello/world".to_owned(),
                                payload: Arc::new(vec![0; 257 * 1024]) };

        match mqtt.handle_outgoing_publish(publish) {
            Err(NetworkError::PacketSizeLimitExceeded) => (),
            _ => panic!("Should throw packet size limit error"),
        }
    }

    #[test]
    fn incoming_publish_should_be_added_to_queue_correctly() {
        let mut mqtt = build_mqttstate();

        // QoS0, 1, 2 Publishes
        let publish1 = build_incoming_publish(QoS::AtMostOnce, 1);
        let publish2 = build_incoming_publish(QoS::AtLeastOnce, 2);
        let publish3 = build_incoming_publish(QoS::ExactlyOnce, 3);

        mqtt.handle_incoming_publish(publish1).unwrap();
        mqtt.handle_incoming_publish(publish2).unwrap();
        mqtt.handle_incoming_publish(publish3).unwrap();

        let pkid = *mqtt.incoming_pub.get(0).unwrap();

        // only qos2 publish should be add to queue
        assert_eq!(mqtt.incoming_pub.len(), 1);
        assert_eq!(pkid, PacketIdentifier(3));
    }

    #[test]
    fn incoming_qos2_publish_should_send_rec_to_network_and_publish_to_user() {
        let mut mqtt = build_mqttstate();
        let publish = build_incoming_publish(QoS::ExactlyOnce, 1);

        let (notification, request) = mqtt.handle_incoming_publish(publish).unwrap();

        match notification {
            Notification::Publish(publish) => assert_eq!(publish.pid.unwrap(), PacketIdentifier(1)),
            _ => panic!("Invalid notification: {:?}", notification),
        }

        match request {
            Request::PubRec(PacketIdentifier(pkid)) => assert_eq!(pkid, 1),
            _ => panic!("Invalid network request: {:?}", request),
        }
    }

    #[test]
    fn incoming_puback_should_remove_correct_publish_from_queue() {
        let mut mqtt = build_mqttstate();

        let publish1 = build_outgoing_publish(QoS::AtLeastOnce);
        let publish2 = build_outgoing_publish(QoS::ExactlyOnce);

        mqtt.handle_outgoing_publish(publish1).unwrap();
        mqtt.handle_outgoing_publish(publish2).unwrap();

        mqtt.handle_incoming_puback(PacketIdentifier(1)).unwrap();
        assert_eq!(mqtt.outgoing_pub.len(), 1);

        let backup = mqtt.outgoing_pub.get(0).unwrap().clone();
        assert_eq!(backup.pid, Some(PacketIdentifier(2)));

        mqtt.handle_incoming_puback(PacketIdentifier(2)).unwrap();
        assert_eq!(mqtt.outgoing_pub.len(), 0);
    }

    #[test]
    fn incoming_pubrec_should_release_correct_publish_from_queue_and_add_releaseid_to_rel_queue() {
        let mut mqtt = build_mqttstate();

        let publish1 = build_outgoing_publish(QoS::AtLeastOnce);
        let publish2 = build_outgoing_publish(QoS::ExactlyOnce);

        let _publish_out = mqtt.handle_outgoing_publish(publish1);
        let _publish_out = mqtt.handle_outgoing_publish(publish2);

        mqtt.handle_incoming_pubrec(PacketIdentifier(2)).unwrap();
        assert_eq!(mqtt.outgoing_pub.len(), 1);

        // check if the remaining element's pkid is 1
        let backup = mqtt.outgoing_pub.get(0).unwrap().clone();
        assert_eq!(backup.pid, Some(PacketIdentifier(1)));

        assert_eq!(mqtt.outgoing_rel.len(), 1);

        // check if the  element's pkid is 2
        let pkid = *mqtt.outgoing_rel.get(0).unwrap();
        assert_eq!(pkid, PacketIdentifier(2));
    }

    #[test]
    fn incoming_pubrec_should_send_release_to_network_and_nothing_to_user() {
        let mut mqtt = build_mqttstate();

        let publish = build_outgoing_publish(QoS::ExactlyOnce);
        mqtt.handle_outgoing_publish(publish).unwrap();

        let (notification, request) = mqtt.handle_incoming_pubrec(PacketIdentifier(1)).unwrap();

        match notification {
            Notification::None => assert!(true),
            _ => panic!("Invalid notification: {:?}", notification),
        }

        match request {
            Request::PubRel(PacketIdentifier(pkid)) => assert_eq!(pkid, 1),
            _ => panic!("Invalid network request: {:?}", request),
        }
    }

    #[test]
    fn incoming_pubrel_should_send_comp_to_network_and_nothing_to_user() {
        let mut mqtt = build_mqttstate();
        let publish = build_incoming_publish(QoS::ExactlyOnce, 1);

        mqtt.handle_incoming_publish(publish).unwrap();
        println!("{:?}", mqtt);
        let (notification, request) = mqtt.handle_incoming_pubrel(PacketIdentifier(1)).unwrap();

        match notification {
            Notification::None => assert!(true),
            _ => panic!("Invalid notification: {:?}", notification),
        }

        match request {
            Request::PubComp(PacketIdentifier(pkid)) => assert_eq!(pkid, 1),
            _ => panic!("Invalid network request: {:?}", request),
        }
    }

    #[test]
    fn incoming_pubcomp_should_release_correct_pkid_from_release_queue() {
        let mut mqtt = build_mqttstate();
        let publish = build_outgoing_publish(QoS::ExactlyOnce);

        mqtt.handle_outgoing_publish(publish).unwrap();
        mqtt.handle_incoming_pubrec(PacketIdentifier(1));
        println!("{:?}", mqtt);

        mqtt.handle_incoming_pubcomp(PacketIdentifier(1)).unwrap();
        assert_eq!(mqtt.outgoing_pub.len(), 0);
    }

    #[test]
    fn outgoing_ping_handle_should_throw_errors_for_no_pingresp() {
        let mut mqtt = build_mqttstate();

        mqtt.opts.keep_alive = Duration::from_secs(5);
        mqtt.connection_status = MqttConnectionStatus::Connected;
        thread::sleep(Duration::from_secs(5));

        // should ping
        assert_eq!((), mqtt.handle_outgoing_ping().unwrap());
        thread::sleep(Duration::from_secs(5));

        // should throw error because we didn't get pingresp for previous ping
        match mqtt.handle_outgoing_ping() {
            Err(NetworkError::AwaitPingResp) => (),
            _ => panic!("Should throw timeout error"),
        }
    }

    #[test]
    fn outgoing_ping_handle_should_throw_error_if_ping_time_exceeded() {
        let mut mqtt = build_mqttstate();

        mqtt.opts.keep_alive = Duration::from_secs(5);
        mqtt.connection_status = MqttConnectionStatus::Connected;
        thread::sleep(Duration::from_secs(7));

        match mqtt.handle_outgoing_ping() {
            Err(NetworkError::Timeout) => (),
            _ => panic!("Should throw timeout error"),
        }
    }

    #[test]
    fn outgoing_ping_handle_should_succeed_if_pingresp_is_received() {
        let mut mqtt = build_mqttstate();

        mqtt.opts.keep_alive = Duration::from_secs(5);
        mqtt.connection_status = MqttConnectionStatus::Connected;
        thread::sleep(Duration::from_secs(5));

        // should ping
        assert_eq!((), mqtt.handle_outgoing_ping().unwrap());
        mqtt.handle_incoming_pingresp().unwrap();
        thread::sleep(Duration::from_secs(5));
        // should ping
        assert_eq!((), mqtt.handle_outgoing_ping().unwrap());
    }

    #[test]
    fn previous_session_handle_should_reset_everything_in_clean_session() {
        let mut mqtt = build_mqttstate();

        mqtt.await_pingresp = true;
        // QoS1 Publish
        let publish = Publish { dup: false,
                                qos: QoS::AtLeastOnce,
                                retain: false,
                                pid: None,
                                topic_name: "hello/world".to_owned(),
                                payload: Arc::new(vec![1, 2, 3]) };

        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish);

        mqtt.handle_previous_session();
        assert_eq!(mqtt.outgoing_pub.len(), 0);
        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Disconnected);
        assert_eq!(mqtt.await_pingresp, false);
    }

    #[test]
    fn previous_session_handle_should_reset_everything_except_queues_in_persistent_session() {
        let mut mqtt = build_mqttstate();

        mqtt.await_pingresp = true;
        mqtt.opts.clean_session = false;
        // QoS1 Publish
        let publish = build_outgoing_publish(QoS::AtLeastOnce);

        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish);

        mqtt.handle_previous_session();
        assert_eq!(mqtt.outgoing_pub.len(), 3);
        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Disconnected);
        assert_eq!(mqtt.await_pingresp, false);
    }

    #[test]
    fn connection_status_is_valid_while_handling_connect_and_connack_packets() {
        let mut mqtt = build_mqttstate();

        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Disconnected);
        mqtt.handle_outgoing_connect().unwrap();
        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Handshake);

        let connack = Connack { session_present: false,
                                code: ConnectReturnCode::Accepted };

        let _ = mqtt.handle_incoming_connack(connack);
        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Connected);

        let connack = Connack { session_present: false,
                                code: ConnectReturnCode::BadUsernamePassword };

        let _ = mqtt.handle_incoming_connack(connack);
        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Disconnected);
    }

    #[test]
    fn connack_handle_should_not_return_list_of_incomplete_messages_to_be_sent_in_clean_session() {
        let mut mqtt = build_mqttstate();

        let publish = Publish { dup: false,
                                qos: QoS::AtLeastOnce,
                                retain: false,
                                pid: None,
                                topic_name: "hello/world".to_owned(),
                                payload: Arc::new(vec![1, 2, 3]) };

        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish);

        let connack = Connack { session_present: false,
                                code: ConnectReturnCode::Accepted };

        mqtt.handle_incoming_connack(connack).unwrap();
        let pubs = mqtt.handle_reconnection();
        assert_eq!(0, pubs.len());
    }

    #[test]
    fn connack_handle_should_return_list_of_incomplete_messages_to_be_sent_in_persistent_session(
        ) {
        let mut mqtt = build_mqttstate();
        mqtt.opts.clean_session = false;

        let publish = build_outgoing_publish(QoS::AtLeastOnce);

        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish);

        let _connack = Connack { session_present: false,
                                 code: ConnectReturnCode::Accepted };

        let pubs = mqtt.handle_reconnection();
        assert_eq!(3, pubs.len());
    }

    #[test]
    fn connect_should_respect_options() {
        use mqttoptions::SecurityOptions::UsernamePassword;

        let lwt = LastWill { topic: String::from("LWT_TOPIC"),
                             message: String::from("LWT_MESSAGE"),
                             qos: QoS::ExactlyOnce,
                             retain: true };

        let opts = MqttOptions::new("test-id", "127.0.0.1", 1883)
            .set_clean_session(true)
            .set_keep_alive(50)
            .set_last_will(lwt.clone())
            .set_security_opts(UsernamePassword((
                String::from("USER"),
                String::from("PASS"),
            )));
        let mut mqtt = MqttState::new(opts);

        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Disconnected);
        let pkt = mqtt.handle_outgoing_connect().unwrap();
        assert_eq!(pkt,
                   Connect { protocol: Protocol::MQTT(4),
                             keep_alive: 50,
                             clean_session: true,
                             client_id: String::from("test-id"),
                             username: Some(String::from("USER")),
                             password: Some(String::from("PASS")),
                             last_will: Some(lwt.clone()) });
    }
}
