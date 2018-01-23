use mqtt3::*;

use std::sync::Arc;

pub fn gen_connect_packet(client_id: String,
                          keep_alive: u16,
                          clean_session: bool,
                          username: Option<String>,
                          password: Option<String>,
                          last_will: Option<LastWill>)
                          -> Connect {
    Connect {
        protocol: Protocol::MQTT(4),
        keep_alive: keep_alive,
        client_id: client_id,
        clean_session: clean_session,
        last_will: last_will,
        username: username,
        password: password,
    }
}

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

pub fn gen_publish_packet(topic_name: String,
                          qos: QoS,
                          pkid: Option<PacketIdentifier>,
                          retain: bool,
                          dup: bool,
                          payload: Arc<Vec<u8>>)
                          -> Publish {
    Publish {
        dup: dup,
        qos: qos,
        retain: retain,
        topic_name: topic_name,
        pid: pkid,
        payload: payload,
    }
}

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