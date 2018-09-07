use std::time::Instant;
use std::collections::VecDeque;
use std::result::Result;

use mqtt3::{Packet, Publish, PacketIdentifier, Connect, Connack, ConnectReturnCode, QoS, Subscribe};
use error::{ConnectError, NetworkError};
use mqttoptions::{MqttOptions, SecurityOptions};
use client::{Notification, Request};

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

    // For QoS 1. Stores outgoing publishes
    outgoing_pub: VecDeque<Publish>,
    // clean_session=false will remember subscriptions only till lives.
    // Even so, if broker crashes, all its state will be lost (most brokers).
    // client should resubscribe it comes back up again or else the data will
    // be lost
    // TODO: Enable this
    // subscriptions: VecDeque<SubscribeTopic>,
}

/// Design: `MqttState` methods will just modify the state of the object
///         but doesn't do any network operations. Methods will do
///         appropriate returns so that n/w methods or n/w eventloop can
///         operate directly. This abstracts the functionality better
///         so that it's easy to switch between synchronous code, tokio (or)
///         async/await

impl MqttState {
    pub fn new(opts: MqttOptions) -> Self {
        MqttState {
            opts,
            connection_status: MqttConnectionStatus::Disconnected,
            await_pingresp: false,
            last_network_activity: Instant::now(),
            last_pkid: PacketIdentifier(0),
            outgoing_pub: VecDeque::new(),
        }
    }

    pub fn handle_outgoing_mqtt_packet(&mut self, packet: Packet) -> Result<Packet, NetworkError> {
        match packet {
            Packet::Publish(publish) => {
                let publish = self.handle_outgoing_publish(publish)?;
                Ok(Packet::Publish(publish))
            },
            Packet::Pingreq => {
                let _ping = self.handle_outgoing_ping()?;
                Ok(Packet::Pingreq)
            }
            Packet::Subscribe(subs) => {
                let subscription = self.handle_outgoing_subscribe(subs)?;
                Ok(Packet::Subscribe(subscription))
            }
            Packet::Disconnect => {
                self.handle_disconnect();
                Ok(Packet::Disconnect)
            },
            Packet::Puback(pkid) => Ok(Packet::Puback(pkid)),
            Packet::Suback(suback) => Ok(Packet::Suback(suback)),
            _ => unimplemented!(),
        }
    }

    // Takes incoming mqtt packet, applies state changes and returns notifiaction packet and
    // network reply packet.
    // Notification packet should be sent to the user and Mqtt reply packet which should be sent
    // back on network
    //
    // E.g For incoming QoS1 publish packet, this method returns (Publish, Puback). Publish packet will
    // be forwarded to user and Pubck packet will be written to network
    pub fn handle_incoming_mqtt_packet(&mut self, packet: Packet) -> Result<(Notification, Request), NetworkError> {
        self.update_last_in_control_time();

        match packet {
            Packet::Pingresp => {
                self.handle_incoming_pingresp();
                Ok((Notification::None, Request::None))
            }
            Packet::Publish(publish) => {
                let notification = Notification::Publish(publish.payload.clone());
                let reply = self.handle_incoming_publish(publish.clone());
                Ok((notification, reply))
            }
            Packet::Suback(_pkid) => {
                let notification = Notification::None;
                let reply = Request::None;
                Ok((notification, reply))
            }
            Packet::Puback(pkid) => {
                self.handle_incoming_puback(pkid)?;
                let notification = Notification::None;
                let reply = Request::None;
                Ok((notification, reply))
            }
            _ => unimplemented!()
        }
    }

    pub fn handle_outgoing_connect(&mut self) -> Result<Connect, ConnectError> {
        self.connection_status = MqttConnectionStatus::Handshake;

        let (_username, _password) = match self.opts.security {
            SecurityOptions::UsernamePassword((ref username, ref password)) => (Some(username.to_owned()), Some(password.to_owned())),
            _ => (None, None),
        };

        Ok(self.opts.connect_packet())
    }

    pub fn handle_incoming_connack(&mut self, connack: Connack) -> Result<(), ConnectError> {
        let response = connack.code;
        if response != ConnectReturnCode::Accepted {
            self.connection_status = MqttConnectionStatus::Disconnected;
            Err(ConnectError::MqttConnectionRefused(response.to_u8()))
        } else {
            self.connection_status = MqttConnectionStatus::Connected;
            if self.opts.clean_session { self.clear_session_info(); }

            Ok(())
        }
    }

    pub fn handle_reconnection(&mut self) -> VecDeque<Packet> {
        if self.opts.clean_session {
            VecDeque::new()
        } else {
            //TODO: Write unittest for checking state during reconnection
            self.outgoing_pub.clone().into_iter().map(|publish| Packet::Publish(publish)).collect()
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
            return Err(NetworkError::PacketSizeLimitExceeded)
        }

        let publish = match publish.qos {
            QoS::AtMostOnce => publish,
            QoS::AtLeastOnce => self.add_packet_id_and_save(publish),
            _ => unimplemented!()
        };

        if self.connection_status == MqttConnectionStatus::Connected {
            Ok(publish)
        } else {
            Err(NetworkError::InvalidState)
        }
    }

    pub fn handle_incoming_puback(&mut self, pkid: PacketIdentifier) -> Result<(), NetworkError> {
        if let Some(index) = self.outgoing_pub.iter().position(|x| x.pid == Some(pkid)) {
            let _publish  = self.outgoing_pub.remove(index).expect("Wrong index");
            Ok(())
        } else {
            error!("Unsolicited PUBLISH packet: {:?}", pkid);
            Err(NetworkError::Unsolicited)
        }
    }

    // return a tuple. tuple.0 is supposed to be send to user through 'notify_tx' while tuple.1
    // should be sent back on network as ack
    pub fn handle_incoming_publish(&mut self, publish: Publish) -> Request {
        let pkid = publish.pid.unwrap();
        let qos = publish.qos;

        match qos {
            QoS::AtMostOnce => Request::None,
            //TODO: Add method in mqtt3 to convert PacketIdentifier to u16
            QoS::AtLeastOnce => Request::PubAck(pkid),
            QoS::ExactlyOnce => unimplemented!()
        }
    }

    // reset the last control packet sent time
    pub fn update_last_in_control_time(&mut self) {
        self.last_network_activity = Instant::now();
    }

    // check if pinging is required based on last flush time
    pub fn is_ping_required(&self) -> bool {
        let in_elapsed = self.last_network_activity.elapsed();

        debug!("Last incoming packet (network activity) before {:?} seconds", in_elapsed.as_secs());
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
            error!("State = {:?}. Shouldn't ping in this state", self.connection_status);
            Err(NetworkError::InvalidState)
        }
    }

    pub fn handle_incoming_pingresp(&mut self) {
        self.await_pingresp = false;
    }

    pub fn handle_outgoing_subscribe(&mut self, mut subscription: Subscribe) -> Result<Subscribe, NetworkError> {
        let pkid = self.next_pkid();

        if self.connection_status == MqttConnectionStatus::Connected {
            subscription.pid = pkid;

            Ok(subscription)
        } else {
            error!("State = {:?}. Shouldn't subscribe in this state", self.connection_status);
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

    pub fn handle_disconnect(&mut self) {
        debug!("Sending disconnect packet to broker. Awaiting qos 1 publishes = {:?}", self.outgoing_pub);
        self.await_pingresp = false;
        self.connection_status = MqttConnectionStatus::Disconnected;

        // remove all the state
        if self.opts.clean_session {
            self.clear_session_info();
        }
    }

    fn clear_session_info(&mut self) {
        self.outgoing_pub.clear();
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

    use super::{MqttState, MqttConnectionStatus};
    use mqtt3::*;
    use mqttoptions::MqttOptions;
    use error::NetworkError;

    #[test]
    fn next_pkid_roll() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883");
        let mut mqtt = MqttState::new(opts);
        let mut pkt_id = PacketIdentifier(0);
        for _ in 0..65536 {
            pkt_id = mqtt.next_pkid();
        }
        assert_eq!(PacketIdentifier(1), pkt_id);
    }

    #[test]
    fn outgoing_publish_handle_should_set_pkid_correctly_and_add_publish_to_queue_correctly() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883");
        let mut mqtt = MqttState::new(opts);
        mqtt.connection_status = MqttConnectionStatus::Connected;

        // QoS0 Publish
        let publish = Publish {
            dup: false,
            qos: QoS::AtMostOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        let publish_out = mqtt.handle_outgoing_publish(publish);
        // pkid shouldn't be added
        assert_eq!(publish_out.unwrap().pid, None);
        // publish shouldn't added to queue
        assert_eq!(mqtt.outgoing_pub.len(), 0);


        // QoS1 Publish
        let publish = Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        let publish_out = mqtt.handle_outgoing_publish(publish.clone());
        // pkid shouldn't be added
        assert_eq!(publish_out.unwrap().pid, Some(PacketIdentifier(1)));
        // publish shouldn't added to queue
        assert_eq!(mqtt.outgoing_pub.len(), 1);

        let publish_out = mqtt.handle_outgoing_publish(publish.clone());
        // pkid shouldn't be added
        assert_eq!(publish_out.unwrap().pid, Some(PacketIdentifier(2)));
        // publish shouldn't added to queue
        assert_eq!(mqtt.outgoing_pub.len(), 2);
    }

    #[test]
    fn outgoing_publish_handle_should_throw_error_in_invalid_state() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883");
        let mut mqtt = MqttState::new(opts);

        let publish = Publish {
            dup: false,
            qos: QoS::AtMostOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        match mqtt.handle_outgoing_publish(publish) {
            Err(NetworkError::InvalidState) => (),
            _ => panic!("Should throw packet size limit error")
        }
    }

    #[test]
    fn outgoing_publish_handle_should_throw_error_when_packetsize_exceeds_max() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883");
        let mut mqtt = MqttState::new(opts);

        let publish = Publish {
            dup: false,
            qos: QoS::AtMostOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![0; 257 * 1024]),
        };

        match mqtt.handle_outgoing_publish(publish) {
            Err(NetworkError::PacketSizeLimitExceeded) => (),
            _ => panic!("Should throw packet size limit error")
        }
    }

    #[test]
    fn incoming_puback_should_remove_correct_publish_from_queue() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883");
        let mut mqtt = MqttState::new(opts);
        // QoS1 Publish
        let publish = Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        let _publish_out = mqtt.handle_outgoing_publish(publish.clone());
        let _publish_out = mqtt.handle_outgoing_publish(publish.clone());
        let _publish_out = mqtt.handle_outgoing_publish(publish);

        {
            assert_eq!(mqtt.outgoing_pub.len(), 3);
            let backup = mqtt.outgoing_pub.get(0).unwrap();
            assert_eq!(backup.pid, Some(PacketIdentifier(1)));
        }

        {
            let _ = mqtt.handle_incoming_puback(PacketIdentifier(1)).unwrap();
            assert_eq!(mqtt.outgoing_pub.len(), 2);
            let backup = mqtt.outgoing_pub.get(0).unwrap();
            assert_eq!(backup.pid, Some(PacketIdentifier(2)));
        }

        {
            let _ = mqtt.handle_incoming_puback(PacketIdentifier(2)).unwrap();
            assert_eq!(mqtt.outgoing_pub.len(), 1);
            let backup = mqtt.outgoing_pub.get(0).unwrap();
            assert_eq!(backup.pid, Some(PacketIdentifier(3)));
        }

        let _ = mqtt.handle_incoming_puback(PacketIdentifier(3)).unwrap();
        assert_eq!(mqtt.outgoing_pub.len(), 0);
    }

    #[test]
    fn outgoing_ping_handle_should_throw_errors_during_invalid_state() {
        // 1. test for invalid state
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883");
        let mut mqtt = MqttState::new(opts);
        mqtt.opts.keep_alive = Duration::from_secs(5);
        thread::sleep(Duration::from_secs(5));
        match mqtt.handle_outgoing_ping() {
            Err(NetworkError::InvalidState) => (),
            _ => panic!("Should throw timeout error")

        }
    }

    #[test]
    fn outgoing_ping_handle_should_throw_errors_for_no_pingresp() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883");
        let mut mqtt = MqttState::new(opts);
        mqtt.opts.keep_alive = Duration::from_secs(5);
        mqtt.connection_status = MqttConnectionStatus::Connected;
        thread::sleep(Duration::from_secs(5));

        // should ping
        assert_eq!((), mqtt.handle_outgoing_ping().unwrap());
        thread::sleep(Duration::from_secs(5));

        // should throw error because we didn't get pingresp for previous ping
        match mqtt.handle_outgoing_ping() {
            Err(NetworkError::AwaitPingResp) => (),
            _ => panic!("Should throw timeout error")

        }
    }

    // #[test]
    fn outgoing_ping_handle_should_throw_error_if_ping_time_exceeded() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883");
        let mut mqtt = MqttState::new(opts);
        mqtt.opts.keep_alive = Duration::from_secs(5);
        mqtt.connection_status = MqttConnectionStatus::Connected;
        thread::sleep(Duration::from_secs(7));

        match mqtt.handle_outgoing_ping() {
            Err(NetworkError::Timeout) => (),
            _ => panic!("Should throw timeout error")

        }
    }

    #[test]
    fn outgoing_ping_handle_should_succeed_if_pingresp_is_received() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883");
        let mut mqtt = MqttState::new(opts);
        mqtt.opts.keep_alive = Duration::from_secs(5);
        mqtt.connection_status = MqttConnectionStatus::Connected;
        thread::sleep(Duration::from_secs(5));

        // should ping
        assert_eq!((), mqtt.handle_outgoing_ping().unwrap());
        mqtt.handle_incoming_pingresp();
        thread::sleep(Duration::from_secs(5));
        // should ping
        assert_eq!((), mqtt.handle_outgoing_ping().unwrap());
    }

    #[test]
    fn disconnect_handle_should_reset_everything_in_clean_session() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883");
        let mut mqtt = MqttState::new(opts);
        mqtt.await_pingresp = true;
        // QoS1 Publish
        let publish = Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish);

        mqtt.handle_disconnect();
        assert_eq!(mqtt.outgoing_pub.len(), 0);
        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Disconnected);
        assert_eq!(mqtt.await_pingresp, false);
    }

    #[test]
    fn disconnect_handle_should_reset_everything_except_queues_in_persistent_session() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883");
        let mut mqtt = MqttState::new(opts);
        mqtt.await_pingresp = true;
        mqtt.opts.clean_session = false;
        // QoS1 Publish
        let publish = Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish);

        mqtt.handle_disconnect();
        assert_eq!(mqtt.outgoing_pub.len(), 3);
        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Disconnected);
        assert_eq!(mqtt.await_pingresp, false);
    }

    #[test]
    fn connection_status_is_valid_while_handling_connect_and_connack_packets() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883");
        let mut mqtt = MqttState::new(opts);

        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Disconnected);
        mqtt.handle_outgoing_connect().unwrap();
        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Handshake);

        let connack = Connack {
            session_present: false,
            code: ConnectReturnCode::Accepted
        };

        let _ = mqtt.handle_incoming_connack(connack);
        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Connected);

        let connack = Connack {
            session_present: false,
            code: ConnectReturnCode::BadUsernamePassword
        };

        let _ = mqtt.handle_incoming_connack(connack);
        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Disconnected);
    }

    #[test]
    fn connack_handle_should_not_return_list_of_incomplete_messages_to_be_sent_in_clean_session() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883");
        let mut mqtt = MqttState::new(opts);

        let publish = Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish);

        let connack = Connack {
            session_present: false,
            code: ConnectReturnCode::Accepted
        };

        mqtt.handle_incoming_connack(connack).unwrap();
        let pubs = mqtt.handle_reconnection();
        assert_eq!(0, pubs.len());
    }

    #[test]
    fn connack_handle_should_return_list_of_incomplete_messages_to_be_sent_in_persistent_session() {
        let mqtt_opts = MqttOptions::new("test-id", "127.0.0.1:1883");
        let mut mqtt = MqttState::new(mqtt_opts);
        mqtt.opts.clean_session = false;

        let publish = Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish.clone());
        let _ = mqtt.handle_outgoing_publish(publish);

        let connack = Connack {
            session_present: false,
            code: ConnectReturnCode::Accepted
        };

        let pubs = mqtt.handle_reconnection();
        assert_eq!(3, pubs.len());
    }

    #[test]
    fn connect_should_respect_options() {
        use mqttoptions::SecurityOptions::UsernamePassword;

        let lwt = LastWill {
            topic: String::from("LWT_TOPIC"),
            message: String::from("LWT_MESSAGE"),
            qos: QoS::ExactlyOnce,
            retain: true,
        };
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883")
            .set_clean_session(true)
            .set_keep_alive(50)
            .set_last_will(lwt.clone())
            .set_security_opts(UsernamePassword((String::from("USER"), String::from("PASS"))));
        let mut mqtt = MqttState::new(opts);

        assert_eq!(mqtt.connection_status, MqttConnectionStatus::Disconnected);
        let pkt = mqtt.handle_outgoing_connect().unwrap();
        assert_eq!(pkt, Connect {
            protocol: Protocol::MQTT(4),
            keep_alive: 50,
            clean_session: true,
            client_id: String::from("test-id"),
            username: Some(String::from("USER")),
            password: Some(String::from("PASS")),
            last_will: Some(lwt.clone())
        });
    }
}