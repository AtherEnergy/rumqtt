use mqtt3::*;

pub fn gen_connect_packet(id: &str, keep_alive: u16, clean_session: bool,
                          username: Option<String>,
                          password: Option<String>)
                          -> Packet {
    Packet::Connect(Connect {
        protocol: Protocol::MQTT(4),
        keep_alive: keep_alive,
        client_id: id.to_string(),
        clean_session: clean_session,
        last_will: None,
        username: username,
        password: password,
    })
}

// pub fn gen_disconnect_packet() -> Packet {
//     Packet::Disconnect
// }

pub fn gen_pingreq_packet() -> Packet {
    Packet::Pingreq
}

// pub fn gen_pingresp_packet() -> Packet {
//     Packet::Pingresp
// }

// pub fn gen_subscribe_packet(pkid: PacketIdentifier, topics: Vec<SubscribeTopic>) -> Packet {
//     Packet::Subscribe(Subscribe {
//         pid: pkid,
//         topics: topics,
//     })
// }

// pub fn gen_unsubscribe_packet(pkid: PacketIdentifier, topics: Vec<String>) -> Packet {
//     Packet::Unsubscribe(Unsubscribe {
//         pid: pkid,
//         topics: topics,
//     })
// }

// pub fn gen_publish_packet(topic_name: String,
//                           qos: QoS,
//                           pkid: Option<PacketIdentifier>,
//                           retain: bool,
//                           dup: bool,
//                           payload: Arc<Vec<u8>>)
//                           -> Packet {
//     Packet::Publish(Publish {
//         dup: dup,
//         qos: qos,
//         retain: retain,
//         topic_name: topic_name,
//         pid: pkid,
//         payload: payload,
//     })
// }

// pub fn gen_puback_packet(pkid: PacketIdentifier) -> Packet {
//     Packet::Puback(pkid)
// }

// pub fn gen_pubrec_packet(pkid: PacketIdentifier) -> Packet {
//     Packet::Pubrec(pkid)
// }

// pub fn gen_pubrel_packet(pkid: PacketIdentifier) -> Packet {
//     Packet::Pubrel(pkid)
// }

// pub fn gen_pubcomp_packet(pkid: PacketIdentifier) -> Packet {
//     Packet::Pubcomp(pkid)
// }