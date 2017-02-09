//! Methods and utilities for generating various packets according to MQTT spec

use std::sync::Arc;
use clientoptions::MqttOptions;

use mqtt3::{Packet, Protocol, Connect, Subscribe, Unsubscribe, Publish, QoS, SubscribeTopic, PacketIdentifier};

pub fn gen_connect_packet(opts: MqttOptions, username: Option<String>, password: Option<String>) -> Packet {
    Packet::Connect(Box::new(Connect {
        protocol: Protocol::MQTT(4),
        keep_alive: opts.keep_alive,
        client_id: opts.client_id,
        clean_session: opts.clean_session,
        last_will: None,
        username: opts.username,
        password: opts.password,
    }))
}

pub fn gen_disconnect_packet() -> Packet {
    Packet::Disconnect
}

pub fn gen_pingreq_packet() -> Packet {
    Packet::Pingreq
}

pub fn gen_pingresp_packet() -> Packet {
    Packet::Pingresp
}

pub fn gen_subscribe_packet(pkid: PacketIdentifier, topics: Vec<SubscribeTopic>) -> Packet {
    Packet::Subscribe(Box::new(Subscribe {
        pid: pkid,
        topics: topics,
    }))
}

pub fn gen_unsubscribe_packet(pkid: PacketIdentifier, topics: Vec<String>) -> Packet {
    Packet::Unsubscribe(Box::new(Unsubscribe {
        pid: pkid,
        topics: topics,
    }))
}

pub fn gen_publish_packet(topic_name: String,
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

pub fn gen_puback_packet(pkid: PacketIdentifier) -> Packet {
    Packet::Puback(pkid)
}

pub fn gen_pubrec_packet(pkid: PacketIdentifier) -> Packet {
    Packet::Pubrec(pkid)
}

pub fn gen_pubrel_packet(pkid: PacketIdentifier) -> Packet {
    Packet::Pubrel(pkid)
}

pub fn gen_pubcomp_packet(pkid: PacketIdentifier) -> Packet {
    Packet::Pubcomp(pkid)
}
