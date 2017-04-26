use mqtt::packet::*;
use mqtt::{Encodable, QualityOfService, TopicFilter};
use mqtt::topic_name::TopicName;

use clientoptions::MqttOptions;

use error::Result;

pub fn generate_connect_packet(opts: MqttOptions) -> Result<Vec<u8>> {
    let mut connect_packet = ConnectPacket::new("MQTT".to_owned(), opts.client_id.unwrap());

    connect_packet.set_clean_session(opts.clean_session);

    if let Some(keep_alive) = opts.keep_alive {
        connect_packet.set_keep_alive(keep_alive);
    }

    // Converting (String, String) -> (TopicName, String)
    let will = match opts.will {
        Some(will) => Some((TopicName::new(will.0)?, will.1.into_bytes())),
        None => None,
    };

    if will.is_some() {
        connect_packet.set_will(will);
        connect_packet.set_will_qos(opts.will_qos as u8);
        connect_packet.set_will_retain(opts.will_retain);
    }

    // mqtt-protocol APIs are directly handling None cases.
    connect_packet.set_user_name(opts.username);
    connect_packet.set_password(opts.password);

    let mut buf = Vec::new();

    connect_packet.encode(&mut buf)?;
    Ok(buf)
}

pub fn generate_disconnect_packet() -> Result<Vec<u8>> {
    let disconnect_packet = DisconnectPacket::new();
    let mut buf = Vec::new();

    disconnect_packet.encode(&mut buf)?;
    Ok(buf)
}

pub fn generate_pingreq_packet() -> Result<Vec<u8>> {
    let pingreq_packet = PingreqPacket::new();
    let mut buf = Vec::new();

    pingreq_packet.encode(&mut buf)?;
    Ok(buf)
}

pub fn generate_subscribe_packet(topics: Vec<(TopicFilter, QualityOfService)>) -> Result<Vec<u8>> {
    let subscribe_packet = SubscribePacket::new(11, topics);
    let mut buf = Vec::new();

    subscribe_packet.encode(&mut buf)?;
    Ok(buf)
}

// TODO: dup flag
pub fn generate_publish_packet(topic: TopicName,
                               qos: QoSWithPacketIdentifier,
                               retain: bool,
                               payload: Vec<u8>)
                               -> Result<Vec<u8>> {
    let mut publish_packet = PublishPacket::new(topic, qos, payload);
    let mut buf = Vec::new();
    publish_packet.set_retain(retain);
    // publish_packet.set_dup(dup);
    publish_packet.encode(&mut buf)?;
    Ok(buf)
}

pub fn generate_puback_packet(pkid: u16) -> Result<Vec<u8>> {
    let puback_packet = PubackPacket::new(pkid);
    let mut buf = Vec::new();

    puback_packet.encode(&mut buf)?;
    Ok(buf)
}

pub fn generate_pubrec_packet(pkid: u16) -> Result<Vec<u8>> {
    let pubrec_packet = PubrecPacket::new(pkid);
    let mut buf = Vec::new();

    pubrec_packet.encode(&mut buf)?;
    Ok(buf)
}

pub fn generate_pubrel_packet(pkid: u16) -> Result<Vec<u8>> {
    let pubrel_packet = PubrelPacket::new(pkid);
    let mut buf = Vec::new();

    pubrel_packet.encode(&mut buf)?;
    Ok(buf)
}

pub fn generate_pubcomp_packet(pkid: u16) -> Result<Vec<u8>> {
    let pubcomp_packet = PubcompPacket::new(pkid);
    let mut buf = Vec::new();

    pubcomp_packet.encode(&mut buf)?;
    Ok(buf)
}
