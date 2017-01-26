use std::sync::Arc;

use mqtt3::{Packet, Protocol, Connect, Subscribe, Unsubscribe, Publish, QoS, SubscribeTopic,
            PacketIdentifier};

pub fn generate_connect_packet(client_id: String,
                               clean_session: bool,
                               keep_alive: u16,
                               username: Option<String>,
                               password: Option<String>)
                               -> Packet {

    Packet::Connect(Box::new(Connect {
        protocol: Protocol::MQTT(4),
        keep_alive: keep_alive,
        client_id: client_id,
        clean_session: clean_session,
        last_will: None,
        username: username,
        password: password,
    }))
}

pub fn generate_disconnect_packet() -> Packet {
    Packet::Disconnect
}

pub fn generate_pingreq_packet() -> Packet {
    Packet::Pingreq
}

pub fn generate_pingresp_packet() -> Packet {
    Packet::Pingresp
}

pub fn generate_subscribe_packet(pkid: PacketIdentifier, topics: Vec<SubscribeTopic>) -> Packet {
    Packet::Subscribe(Box::new(Subscribe {
        pid: pkid,
        topics: topics,
    }))
}

pub fn generate_unsubscribe_packet(pkid: PacketIdentifier, topics: Vec<String>) -> Packet {
    Packet::Unsubscribe(Box::new(Unsubscribe {
        pid: pkid,
        topics: topics,
    }))
}

pub fn generate_publish_packet(topic_name: String,
                               qos: QoS,
                               pkid: Option<PacketIdentifier>,
                               retain: bool,
                               dup: bool,
                               payload: Arc<Vec<u8>>)
                               -> Packet {
    Packet::Publish(Box::new(Publish {
        dup: dup,
        qos: qos,
        retain: retain,
        topic_name: topic_name,
        pid: pkid,
        payload: payload,
    }))
}

pub fn generate_puback_packet(pkid: PacketIdentifier) -> Packet {
    Packet::Puback(pkid)
}

pub fn generate_pubrec_packet(pkid: PacketIdentifier) -> Packet {
    Packet::Pubrec(pkid)
}

pub fn generate_pubrel_packet(pkid: PacketIdentifier) -> Packet {
    Packet::Pubrel(pkid)
}

pub fn generate_pubcomp_packet(pkid: PacketIdentifier) -> Packet {
    Packet::Pubcomp(pkid)
}
