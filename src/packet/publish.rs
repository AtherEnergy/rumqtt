use std::io::{Read, Write};

use control::{FixedHeader, PacketType, ControlType};
use control::variable_header::PacketIdentifier;
use packet::{Packet, PacketError};
use topic_name::TopicName;
use {Encodable, Decodable};

#[derive(Debug)]
pub enum QoSWithPacketIdentifier {
    Level0,
    Level1(u16),
    Level2(u16),
}

#[derive(Debug, Eq, PartialEq)]
pub struct PublishPacket {
    fixed_header: FixedHeader,
    topic_name: TopicName,
    packet_identifier: Option<PacketIdentifier>,
    payload: Vec<u8>,
}

impl PublishPacket {
    pub fn new(topic_name: TopicName,
               qos: QoSWithPacketIdentifier,
               payload: Vec<u8>)
               -> PublishPacket {
        let (qos, pkid) = match qos {
            QoSWithPacketIdentifier::Level0 => (0, None),
            QoSWithPacketIdentifier::Level1(pkid) => (1, Some(PacketIdentifier(pkid))),
            QoSWithPacketIdentifier::Level2(pkid) => (2, Some(PacketIdentifier(pkid))),
        };

        let mut pk = PublishPacket {
            fixed_header: FixedHeader::new(PacketType::with_default(ControlType::Publish), 0),
            topic_name: topic_name,
            packet_identifier: pkid,
            payload: payload,
        };
        pk.fixed_header.packet_type.flags |= qos << 1;
        pk.fixed_header.remaining_length = pk.calculate_remaining_length();
        pk
    }

    #[inline]
    fn calculate_remaining_length(&self) -> u32 {
        self.encoded_variable_headers_length() + self.payload().encoded_length()
    }

    pub fn set_dup(&mut self, dup: bool) {
        self.fixed_header.packet_type.flags |= (dup as u8) << 3;
    }

    pub fn dup(&self) -> bool {
        self.fixed_header.packet_type.flags & 0x80 != 0
    }

    pub fn set_qos(&mut self, qos: QoSWithPacketIdentifier) {
        let (qos, pkid) = match qos {
            QoSWithPacketIdentifier::Level0 => (0, None),
            QoSWithPacketIdentifier::Level1(pkid) => (1, Some(PacketIdentifier(pkid))),
            QoSWithPacketIdentifier::Level2(pkid) => (2, Some(PacketIdentifier(pkid))),
        };
        self.fixed_header.packet_type.flags |= qos << 1;
        self.packet_identifier = pkid;
    }

    pub fn qos(&self) -> QoSWithPacketIdentifier {
        match self.packet_identifier {
            None => QoSWithPacketIdentifier::Level0,
            Some(pkid) => {
                let qos_val = (self.fixed_header.packet_type.flags & 0x06) >> 1;
                match qos_val {
                    1 => QoSWithPacketIdentifier::Level1(pkid.0),
                    2 => QoSWithPacketIdentifier::Level2(pkid.0),
                    _ => unreachable!(),
                }
            }
        }
    }

    pub fn set_retain(&mut self, ret: bool) {
        self.fixed_header.packet_type.flags |= ret as u8;
    }

    pub fn retain(&self) -> bool {
        self.fixed_header.packet_type.flags & 0x01 != 0
    }

    pub fn set_topic_name(&mut self, topic_name: TopicName) {
        self.topic_name = topic_name;
        self.fixed_header.remaining_length = self.calculate_remaining_length();
    }

    pub fn topic_name(&self) -> &str {
        &self.topic_name[..]
    }
}

impl<'a> Packet<'a> for PublishPacket {
    type Payload = Vec<u8>;

    fn fixed_header(&self) -> &FixedHeader {
        &self.fixed_header
    }

    fn payload(&self) -> &Self::Payload {
        &self.payload
    }

    fn encode_variable_headers<W: Write>(&self,
                                         writer: &mut W)
                                         -> Result<(), PacketError<'a, Self>> {
        try!(self.topic_name.encode(writer));

        if let Some(pkid) = self.packet_identifier.as_ref() {
            try!(pkid.encode(writer));
        }

        Ok(())
    }

    fn encoded_variable_headers_length(&self) -> u32 {
        self.topic_name.encoded_length() +
        self.packet_identifier.as_ref().map(|x| x.encoded_length()).unwrap_or(0)
    }

    fn decode_packet<R: Read>(reader: &mut R,
                              fixed_header: FixedHeader)
                              -> Result<Self, PacketError<'a, Self>> {
        let topic_name: TopicName = try!(TopicName::decode(reader));

        let packet_identifier = if fixed_header.packet_type.flags & 0x06 != 0 {
            Some(try!(PacketIdentifier::decode(reader)))
        } else {
            None
        };

        let vhead_len = topic_name.encoded_length() +
                        packet_identifier.as_ref().map(|x| x.encoded_length()).unwrap_or(0);
        let payload_len = fixed_header.remaining_length - vhead_len;

        let payload: Vec<u8> = try!(Decodable::decode_with(reader, Some(payload_len)));

        Ok(PublishPacket {
            fixed_header: fixed_header,
            topic_name: topic_name,
            packet_identifier: packet_identifier,
            payload: payload,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use std::io::Cursor;

    use topic_name::TopicName;
    use {Encodable, Decodable};

    #[test]
    fn test_publish_packet_basic() {
        let packet = PublishPacket::new(TopicName::new("a/b".to_owned()).unwrap(),
                                        QoSWithPacketIdentifier::Level2(10),
                                        b"Hello world!".to_vec());

        let mut buf = Vec::new();
        packet.encode(&mut buf).unwrap();

        let mut decode_buf = Cursor::new(buf);
        let decoded = PublishPacket::decode(&mut decode_buf).unwrap();

        assert_eq!(packet, decoded);
    }
}
