use std::time::{Duration, Instant};
use std::collections::VecDeque;
use std::result::Result;
use std::path::Path;

use failure;
use mqtt3::{Packet, Publish, PacketIdentifier, Connect, Connack, ConnectReturnCode, QoS, Subscribe};

use error::{PingError, ConnectError, PublishError, PubackError, SubscribeError};
use packet;
use MqttOptions;
use SecurityOptions;

type Notification = Packet;
type Reply = Packet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MqttConnectionStatus {
    Handshake,
    Connected,
    Disconnected,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    iat: i64,
    exp: i64,
    aud: String,
}

#[derive(Debug)]
pub struct MqttState {
    opts: MqttOptions,
    
    // --------  State  ----------
    connection_status: MqttConnectionStatus,
    await_pingresp: bool,
    pub last_out_control_time: Instant,
    pub last_in_control_time: Instant,
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
            opts: opts,
            connection_status: MqttConnectionStatus::Disconnected,
            await_pingresp: false,
            last_out_control_time: Instant::now(),
            last_in_control_time: Instant::now(),
            last_pkid: PacketIdentifier(0),
            outgoing_pub: VecDeque::new(),
            // subscriptions: VecDeque::new(),
        }
    }

    pub fn handle_outgoing_mqtt_packet(&mut self, packet: Packet) -> Result<Packet, failure::Error> {
        self.update_last_out_control_time();

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
    pub fn handle_incoming_mqtt_packet(&mut self, packet: Packet) -> Result<(Option<Notification>, Option<Reply>), failure::Error> {        
        self.update_last_in_control_time();

        match packet {
            Packet::Connack(connack) => {
                    self.handle_incoming_connack(connack)?;
                    Ok((None, None))
            }
            Packet::Puback(ack) => {
                // ignore unsolicited ack errors
                let _ = self.handle_incoming_puback(ack);
                Ok((Some(Packet::Puback(ack)), None))
            }
            Packet::Pingresp => {
                self.handle_incoming_pingresp();
                Ok((None, None))
            }
            Packet::Publish(publish) => {
                let ack = self.handle_incoming_publish(publish.clone());
                let publish = Some(Packet::Publish(publish));
                Ok((publish, ack))
            }
            Packet::Suback(suback) => Ok((Some(Packet::Suback(suback)), None)),
            _ => unimplemented!()
        }
    }

    pub fn handle_outgoing_connect(&mut self) -> Result<Connect, ConnectError> {
        let keep_alive = if let Some(keep_alive) = self.opts.keep_alive {
            keep_alive
        } else {
            // rumqtt sets keep alive time to 2 minutes if user sets it to none.
            // (get consensus)
            Duration::new(120, 0)
        };

        self.opts.keep_alive = Some(keep_alive);
        self.connection_status = MqttConnectionStatus::Handshake;

        let (username, password) = match self.opts.security {
            SecurityOptions::UsernamePassword((ref username, ref password)) => (Some(username.to_owned()), Some(password.to_owned())),
            #[cfg(feature = "jwt")]
            SecurityOptions::GcloudIotCore((ref project, ref key, expiry)) => (Some("unused".to_owned()), Some(gen_iotcore_password(project, key, expiry)?)),
            _ => (None, None),
        };

        Ok(packet::gen_connect_packet(self.opts.client_id.clone(), keep_alive, self.opts.clean_session, username, password, self.opts.last_will.clone()))
    }

    pub fn handle_incoming_connack(&mut self, connack: Connack) -> Result<(), ConnectError> {
        let response = connack.code;
        if response != ConnectReturnCode::Accepted {
            self.connection_status = MqttConnectionStatus::Disconnected;
            Err(ConnectError::MqttConnectionRefused(response.to_u8()))
        } else {
            self.connection_status = MqttConnectionStatus::Connected;
            
            if self.opts.clean_session {
                self.clear_session_info();
            }

            Ok(())
        }
    }

    pub fn handle_reconnection(&mut self) -> VecDeque<Packet> {
        if self.opts.clean_session {
            VecDeque::new()
        } else {
            self.outgoing_pub.clone().into_iter().map(Packet::Publish).collect()
        }
    }

    /// Sets next packet id if pkid is None (fresh publish) and adds it to the
    /// outgoing publish queue
    pub fn handle_outgoing_publish(&mut self, mut publish: Publish) -> Result<Publish, PublishError> {
        if publish.payload.len() > self.opts.max_packet_size {
            return Err(PublishError::PacketSizeLimitExceeded)
        }

        let publish = match publish.qos {
            QoS::AtMostOnce => publish,
            QoS::AtLeastOnce => {
                // add pkid if None
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
            _ => unimplemented!()
        };

        if self.connection_status == MqttConnectionStatus::Connected {
            Ok(publish)
        } else {
            Err(PublishError::InvalidState)
        }
    }

    pub fn handle_incoming_puback(&mut self, pkid: PacketIdentifier) -> Result<(), PubackError> {
        if let Some(index) = self.outgoing_pub.iter().position(|x| x.pid == Some(pkid)) {
            let _ = self.outgoing_pub.remove(index);
            Ok(())
        } else {
            error!("Unsolicited PUBLISH packet: {:?}", pkid);
            Err(PubackError::Unsolicited)
        }
    }

    // return a tuple. tuple.0 is supposed to be send to user through 'notify_tx' while tuple.1
    // should be sent back on network as ack
    pub fn handle_incoming_publish(&mut self, publish: Publish) -> Option<Packet> {
        let pkid = publish.pid;
        let qos = publish.qos;

        match qos {
            QoS::AtMostOnce => None,
            QoS::AtLeastOnce => Some(Packet::Puback(pkid.unwrap())),
            QoS::ExactlyOnce => unimplemented!()
        }
    }

    // reset the last control packet sent time
    pub fn update_last_out_control_time(&mut self) {
        self.last_out_control_time = Instant::now();
    }

    // reset the last control packet sent time
    pub fn update_last_in_control_time(&mut self) {
        self.last_in_control_time = Instant::now();
    }

    // check if pinging is required based on last flush time
    pub fn is_ping_required(&self) -> bool {
        if let Some(keep_alive) = self.opts.keep_alive  {
            let out_elapsed = self.last_out_control_time.elapsed();
            let in_elapsed = self.last_in_control_time.elapsed();
            
            debug!("Last outgoing before {:?} seconds. Last incoming packet before {:?} seconds", out_elapsed.as_secs(), in_elapsed.as_secs());
            out_elapsed >= keep_alive || in_elapsed >= keep_alive
        } else {
            false
        }
    }

    // check when the last control packet/pingreq packet
    // is received and return the status which tells if
    // keep alive time has exceeded
    // NOTE: status will be checked for zero keepalive times also
    pub fn handle_outgoing_ping(&mut self) -> Result<(), PingError> {
        // let keep_alive = self.opts.keep_alive.expect("No keep alive");

        // let elapsed = self.last_out_control_time.elapsed();
        // if elapsed >= keep_alive {
        //     error!("Elapsed time {:?} is greater than keep alive {:?}. Timeout error", elapsed.as_secs(), keep_alive);
        //     return Err(PingError::Timeout);
        // }
        // @ Prevents half open connections. Tcp writes will buffer up
        // with out throwing any error (till a timeout) when internet
        // is down. Eventhough broker closes the socket after timeout,
        // EOF will be known only after reconnection.
        // We need to unbind the socket if there in no pingresp before next ping
        // (What about case when pings aren't sent because of constant publishes
        // ?. A. Tcp write buffer gets filled up and write will be blocked for 10
        // secs and then error out because of timeout.)

        // raise error if last ping didn't receive ack
        if self.await_pingresp {
            error!("Error awaiting for last ping response");
            return Err(PingError::AwaitPingResp);
        }

        if self.connection_status == MqttConnectionStatus::Connected {
            self.await_pingresp = true;
            Ok(())
        } else {
            error!("State = {:?}. Shouldn't ping in this state", self.connection_status);
            Err(PingError::InvalidState)
        }
    }

    pub fn handle_incoming_pingresp(&mut self) {
        self.await_pingresp = false;
    }

    pub fn handle_outgoing_subscribe(&mut self, mut subscription: Subscribe) -> Result<Subscribe, SubscribeError> {
        let pkid = self.next_pkid();

        if self.connection_status == MqttConnectionStatus::Connected {
            subscription.pid = pkid;

            Ok(subscription)
        } else {
            error!("State = {:?}. Shouldn't subscribe in this state", self.connection_status);
            Err(SubscribeError::InvalidState)
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
        self.await_pingresp = false;
        self.connection_status = MqttConnectionStatus::Disconnected;

        // remove all the state
        if self.opts.clean_session {
            self.clear_session_info();
        }
    }

    fn clear_session_info(&mut self) {
        self.outgoing_pub.clear();
        self.last_out_control_time = Instant::now();
        self.last_in_control_time = Instant::now();
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

#[cfg(feature = "jwt")]
// Generates a new password for mqtt client authentication
pub fn gen_iotcore_password<P>(project: &str, key: P, expiry: i64) -> Result<String, ConnectError>
where P: AsRef<Path> {
    use std::fs::File;
    use std::io::Read;

    use jwt::{encode, Header, Algorithm};
    use chrono::{self, Utc};

    let time = Utc::now();
    let jwt_header = Header::new(Algorithm::RS256);
    let iat = time.timestamp();
    let exp = time.checked_add_signed(chrono::Duration::minutes(expiry)).expect("Unable to create expiry").timestamp();
    let claims = Claims {
        iat: iat,
        exp: exp,
        aud: project.to_owned(),
    };

    let mut key_file = File::open(key)?;
    let mut key = vec![];
    key_file.read_to_end(&mut key)?;
    Ok(encode(&jwt_header, &claims, &key)?)
}

#[cfg(test)]
mod test {
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use super::{MqttState, MqttConnectionStatus};
    use mqtt3::*;
    use mqttopts::MqttOptions;
    use error::{PingError, PublishError};

    #[test]
    fn next_pkid_roll() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883").unwrap();
        let mut mqtt = MqttState::new(opts);
        let mut pkt_id = PacketIdentifier(0);
        for _ in 0..65536 {
            pkt_id = mqtt.next_pkid();
        }
        assert_eq!(PacketIdentifier(1), pkt_id);
    }

    #[test]
    fn outgoing_publish_handle_should_set_pkid_correctly_and_add_publish_to_queue_correctly() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883").unwrap();
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
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883").unwrap();
        let mut mqtt = MqttState::new(opts);

        let publish = Publish {
            dup: false,
            qos: QoS::AtMostOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![1, 2, 3]),
        };

        let publish_out = mqtt.handle_outgoing_publish(publish);
        assert_eq!(publish_out, Err(PublishError::InvalidState));
    }

    #[test]
    fn outgoing_publish_handle_should_throw_error_when_packetsize_exceeds_max() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883").unwrap();
        let mut mqtt = MqttState::new(opts);

        let publish = Publish {
            dup: false,
            qos: QoS::AtMostOnce,
            retain: false,
            pid: None,
            topic_name: "hello/world".to_owned(),
            payload: Arc::new(vec![0; 257 * 1024]),
        };

        let publish_out = mqtt.handle_outgoing_publish(publish);
        assert_eq!(publish_out, Err(PublishError::PacketSizeLimitExceeded));
    }

    #[test]
    fn incoming_puback_should_remove_correct_publish_from_queue() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883").unwrap();
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

        let publish_out = mqtt.handle_outgoing_publish(publish.clone());
        let publish_out = mqtt.handle_outgoing_publish(publish.clone());
        let publish_out = mqtt.handle_outgoing_publish(publish);
        
        {
            assert_eq!(mqtt.outgoing_pub.len(), 3);
            let publish_pack = mqtt.outgoing_pub.get(0).unwrap();
            assert_eq!(publish_pack.pid, Some(PacketIdentifier(1)));
        }
        let publish = mqtt.handle_incoming_puback(PacketIdentifier(1)).unwrap();
        assert_eq!(mqtt.outgoing_pub.len(), 2);

        {
            assert_eq!(mqtt.outgoing_pub.len(), 2);
            let publish_pack = mqtt.outgoing_pub.get(0).unwrap();
            assert_eq!(publish_pack.pid, Some(PacketIdentifier(2)));
        }
        let publish = mqtt.handle_incoming_puback(PacketIdentifier(2)).unwrap();
        assert_eq!(mqtt.outgoing_pub.len(), 1);

        {
            assert_eq!(mqtt.outgoing_pub.len(), 1);
            let publish_pack = mqtt.outgoing_pub.get(0).unwrap();
            assert_eq!(publish_pack.pid, Some(PacketIdentifier(3)));
        }
        let publish = mqtt.handle_incoming_puback(PacketIdentifier(3)).unwrap();
        assert_eq!(mqtt.outgoing_pub.len(), 0);
    }

    #[test]
    fn outgoing_ping_handle_should_throw_errors_during_invalid_state() {
        // 1. test for invalid state
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883").unwrap();
        let mut mqtt = MqttState::new(opts);
        mqtt.opts.keep_alive = Some(5);
        thread::sleep(Duration::new(5, 0));
        assert_eq!(Err(PingError::InvalidState), mqtt.handle_outgoing_ping());
    }

    #[test]
    fn outgoing_ping_handle_should_throw_errors_for_no_pingresp() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883").unwrap();
        let mut mqtt = MqttState::new(opts);
        mqtt.opts.keep_alive = Some(5);
        mqtt.connection_status = MqttConnectionStatus::Connected;
        thread::sleep(Duration::new(5, 0));
        // should ping
        assert_eq!(Ok(()), mqtt.handle_outgoing_ping());
        thread::sleep(Duration::new(5, 0));
        // should throw error because we didn't get pingresp for previous ping
        assert_eq!(Err(PingError::AwaitPingResp), mqtt.handle_outgoing_ping());
    }

    #[test]
    fn outgoing_ping_handle_should_throw_error_if_ping_time_exceeded() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883").unwrap();
        let mut mqtt = MqttState::new(opts);
        mqtt.opts.keep_alive = Some(5);
        mqtt.connection_status = MqttConnectionStatus::Connected;
        thread::sleep(Duration::new(7, 0));
        // should ping
        assert_eq!(Err(PingError::Timeout), mqtt.handle_outgoing_ping());
    }

    #[test]
    fn outgoing_ping_handle_should_succeed_if_pingresp_is_received() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883").unwrap();
        let mut mqtt = MqttState::new(opts);
        mqtt.opts.keep_alive = Some(5);
        mqtt.connection_status = MqttConnectionStatus::Connected;
        thread::sleep(Duration::new(5, 0));
        // should ping
        assert_eq!(Ok(()), mqtt.handle_outgoing_ping());
        mqtt.handle_incoming_pingresp();
        thread::sleep(Duration::new(5, 0));
        // should ping
        assert_eq!(Ok(()), mqtt.handle_outgoing_ping());
    }

    #[test]
    fn disconnect_handle_should_reset_everything_in_clean_session() {
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883").unwrap();
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
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883").unwrap();
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
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883").unwrap();
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
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883").unwrap();
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
        let mqtt_opts = MqttOptions::new("test-id", "127.0.0.1:1883").unwrap();
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
    fn connec_should_respect_options() {
        use mqttopts::SecurityOptions::UsernamePassword;

        let lwt = LastWill {
            topic: String::from("LWT_TOPIC"),
            message: String::from("LWT_MESSAGE"),
            qos: QoS::ExactlyOnce,
            retain: true,
        };
        let opts = MqttOptions::new("test-id", "127.0.0.1:1883")
            .unwrap()
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