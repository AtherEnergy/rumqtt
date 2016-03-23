use std::io::{self, Read, Write};
use std::error::Error;
use std::fmt;


use control::{FixedHeader, PacketType, ControlType};
use control::variable_header::{ProtocolName, ProtocolLevel, ConnectFlags, KeepAlive};
use control::variable_header::protocol_level::SPEC_3_1_1;
use packet::{Packet, PacketError};
use topic_name::{TopicName, TopicNameError};
use {Encodable, Decodable};
use encodable::StringEncodeError;

#[derive(Debug, Eq, PartialEq)]
pub struct ConnectPacket {
    fixed_header: FixedHeader,
    protocol_name: ProtocolName,

    protocol_level: ProtocolLevel,
    flags: ConnectFlags,
    keep_alive: KeepAlive,

    payload: ConnectPacketPayload,
}

impl ConnectPacket {
    pub fn new(protoname: String, client_identifier: String) -> ConnectPacket {
        ConnectPacket::with_level(protoname, client_identifier, SPEC_3_1_1)
    }

    pub fn with_level(protoname: String, client_identifier: String, level: u8) -> ConnectPacket {
        let mut pk = ConnectPacket {
            fixed_header: FixedHeader::new(PacketType::with_default(ControlType::Connect), 0),
            protocol_name: ProtocolName(protoname),
            protocol_level: ProtocolLevel(level),
            flags: ConnectFlags::empty(),
            keep_alive: KeepAlive(3),
            payload: ConnectPacketPayload::new(client_identifier),
        };

        pk.fixed_header.remaining_length = pk.calculate_remaining_length();

        pk
    }

    #[inline]
    fn calculate_remaining_length(&self) -> u32 {
        self.encoded_variable_headers_length() + self.payload().encoded_length()
    }

    pub fn set_user_name(&mut self, name: Option<String>) {
        self.flags.user_name = name.is_some();
        self.payload.user_name = name;
        self.fixed_header.remaining_length = self.calculate_remaining_length();
    }

    pub fn set_will(&mut self, topic_message: Option<(TopicName, String)>) {
        self.flags.will_flag = topic_message.is_some();

        match topic_message {
            Some((topic, msg)) => {
                self.payload.will_topic = Some(topic);
                self.payload.will_message = Some(msg);
            }
            None => {
                self.payload.will_topic = None;
                self.payload.will_message = None;
            }
        }

        self.fixed_header.remaining_length = self.calculate_remaining_length();
    }

    pub fn set_password(&mut self, password: Option<String>) {
        self.flags.password = password.is_some();
        self.payload.password = password;
        self.fixed_header.remaining_length = self.calculate_remaining_length();
    }

    pub fn set_client_identifier(&mut self, id: String) {
        self.payload.client_identifier = id;
        self.fixed_header.remaining_length = self.calculate_remaining_length();
    }

    pub fn set_will_retain(&mut self, will_retain: bool) {
        self.flags.will_retain = will_retain;
    }

    pub fn set_will_qos(&mut self, will_qos: u8) {
        assert!(will_qos <= 2);
        self.flags.will_qos = will_qos;
    }

    pub fn set_clean_session(&mut self, clean_session: bool) {
        self.flags.clean_session = clean_session;
    }

    pub fn user_name(&self) -> Option<&str> {
        self.payload.user_name.as_ref().map(|x| &x[..])
    }

    pub fn password(&self) -> Option<&str> {
        self.payload.password.as_ref().map(|x| &x[..])
    }

    pub fn will(&self) -> Option<(&str, &str)> {
        self.payload
            .will_topic
            .as_ref()
            .map(|x| &x[..])
            .and_then(|topic| {
                self.payload
                    .will_message
                    .as_ref()
                    .map(|x| &x[..])
                    .map(|msg| (topic, msg))
            })
    }

    pub fn will_retain(&self) -> bool {
        self.flags.will_retain
    }

    pub fn will_qos(&self) -> u8 {
        self.flags.will_qos
    }

    pub fn client_identifier(&self) -> &str {
        &self.payload.client_identifier[..]
    }

    pub fn clean_session(&self) -> bool {
        self.flags.clean_session
    }
}

impl<'a> Packet<'a> for ConnectPacket {
    type Payload = ConnectPacketPayload;

    fn fixed_header(&self) -> &FixedHeader {
        &self.fixed_header
    }

    fn payload(&self) -> &ConnectPacketPayload {
        &self.payload
    }

    fn encode_variable_headers<W: Write>(&self,
                                         writer: &mut W)
                                         -> Result<(), PacketError<'a, Self>> {
        try!(self.protocol_name.encode(writer));
        try!(self.protocol_level.encode(writer));
        try!(self.flags.encode(writer));
        try!(self.keep_alive.encode(writer));

        Ok(())
    }

    fn encoded_variable_headers_length(&self) -> u32 {
        self.protocol_name.encoded_length() + self.protocol_level.encoded_length() +
        self.flags.encoded_length() + self.keep_alive.encoded_length()
    }

    fn decode_packet<R: Read>(reader: &mut R,
                              fixed_header: FixedHeader)
                              -> Result<Self, PacketError<'a, Self>> {
        let protoname: ProtocolName = try!(Decodable::decode(reader));
        let protocol_level: ProtocolLevel = try!(Decodable::decode(reader));
        let flags: ConnectFlags = try!(Decodable::decode(reader));
        let keep_alive: KeepAlive = try!(Decodable::decode(reader));
        let payload: ConnectPacketPayload = try!(Decodable::decode_with(reader, Some(&flags))
                                                     .map_err(PacketError::PayloadError));

        Ok(ConnectPacket {
            fixed_header: fixed_header,
            protocol_name: protoname,
            protocol_level: protocol_level,
            flags: flags,
            keep_alive: keep_alive,
            payload: payload,
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct ConnectPacketPayload {
    client_identifier: String,
    will_topic: Option<TopicName>,
    will_message: Option<String>,
    user_name: Option<String>,
    password: Option<String>,
}

impl ConnectPacketPayload {
    pub fn new(client_identifier: String) -> ConnectPacketPayload {
        ConnectPacketPayload {
            client_identifier: client_identifier,
            will_topic: None,
            will_message: None,
            user_name: None,
            password: None,
        }
    }
}

impl<'a> Encodable<'a> for ConnectPacketPayload {
    type Err = ConnectPacketPayloadError;

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), ConnectPacketPayloadError> {
        try!(self.client_identifier.encode(writer));

        if let Some(ref will_topic) = self.will_topic {
            try!(will_topic.encode(writer));
        }

        if let Some(ref will_message) = self.will_message {
            try!(will_message.encode(writer));
        }

        if let Some(ref user_name) = self.user_name {
            try!(user_name.encode(writer));
        }

        if let Some(ref password) = self.password {
            try!(password.encode(writer));
        }

        Ok(())
    }

    fn encoded_length(&self) -> u32 {
        self.client_identifier.encoded_length() +
        self.will_topic.as_ref().map(|t| t.encoded_length()).unwrap_or(0) +
        self.will_message.as_ref().map(|t| t.encoded_length()).unwrap_or(0) +
        self.user_name.as_ref().map(|t| t.encoded_length()).unwrap_or(0) +
        self.password.as_ref().map(|t| t.encoded_length()).unwrap_or(0)
    }
}

impl<'a> Decodable<'a> for ConnectPacketPayload {
    type Err = ConnectPacketPayloadError;
    type Cond = &'a ConnectFlags;

    fn decode_with<R: Read>(reader: &mut R,
                            rest: Option<&'a ConnectFlags>)
                            -> Result<ConnectPacketPayload, ConnectPacketPayloadError> {
        let mut need_will_topic = false;
        let mut need_will_message = false;
        let mut need_user_name = false;
        let mut need_password = false;

        if let Some(r) = rest {
            need_will_topic = r.will_flag;
            need_will_message = r.will_flag;
            need_user_name = r.user_name;
            need_password = r.password;
        }

        let ident: String = try!(Decodable::decode(reader));
        let topic = if need_will_topic {
            Some(try!(Decodable::decode(reader)))
        } else {
            None
        };
        let msg = if need_will_message {
            Some(try!(Decodable::decode(reader)))
        } else {
            None
        };
        let uname = if need_user_name {
            Some(try!(Decodable::decode(reader)))
        } else {
            None
        };
        let pwd = if need_password {
            Some(try!(Decodable::decode(reader)))
        } else {
            None
        };

        Ok(ConnectPacketPayload {
            client_identifier: ident,
            will_topic: topic,
            will_message: msg,
            user_name: uname,
            password: pwd,
        })
    }
}

#[derive(Debug)]
pub enum ConnectPacketPayloadError {
    IoError(io::Error),
    StringEncodeError(StringEncodeError),
    TopicNameError(TopicNameError),
}

impl fmt::Display for ConnectPacketPayloadError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &ConnectPacketPayloadError::IoError(ref err) => err.fmt(f),
            &ConnectPacketPayloadError::StringEncodeError(ref err) => err.fmt(f),
            &ConnectPacketPayloadError::TopicNameError(ref err) => err.fmt(f),
        }
    }
}

impl Error for ConnectPacketPayloadError {
    fn description(&self) -> &str {
        match self {
            &ConnectPacketPayloadError::IoError(ref err) => err.description(),
            &ConnectPacketPayloadError::StringEncodeError(ref err) => err.description(),
            &ConnectPacketPayloadError::TopicNameError(ref err) => err.description(),
        }
    }

    fn cause(&self) -> Option<&Error> {
        match self {
            &ConnectPacketPayloadError::IoError(ref err) => Some(err),
            &ConnectPacketPayloadError::StringEncodeError(ref err) => Some(err),
            &ConnectPacketPayloadError::TopicNameError(ref err) => Some(err),
        }
    }
}

impl From<StringEncodeError> for ConnectPacketPayloadError {
    fn from(err: StringEncodeError) -> ConnectPacketPayloadError {
        ConnectPacketPayloadError::StringEncodeError(err)
    }
}

impl From<TopicNameError> for ConnectPacketPayloadError {
    fn from(err: TopicNameError) -> ConnectPacketPayloadError {
        ConnectPacketPayloadError::TopicNameError(err)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use std::io::Cursor;

    use {Encodable, Decodable};

    #[test]
    fn test_connect_packet_encode_basic() {
        let packet = ConnectPacket::new("MQTT".to_owned(), "12345".to_owned());
        let expected = b"\x10\x11\x00\x04MQTT\x04\x00\x00\x00\x00\x0512345";

        let mut buf = Vec::new();
        packet.encode(&mut buf).unwrap();

        assert_eq!(&expected[..], &buf[..]);
    }

    #[test]
    fn test_connect_packet_decode_basic() {
        let encoded_data = b"\x10\x11\x00\x04MQTT\x04\x00\x00\x00\x00\x0512345";

        let mut buf = Cursor::new(&encoded_data[..]);
        let packet = ConnectPacket::decode(&mut buf).unwrap();

        let expected = ConnectPacket::new("MQTT".to_owned(), "12345".to_owned());
        assert_eq!(expected, packet);
    }

    #[test]
    fn test_connect_packet_user_name() {
        let mut packet = ConnectPacket::new("MQTT".to_owned(), "12345".to_owned());
        packet.set_user_name(Some("mqtt_player".to_owned()));

        let mut buf = Vec::new();
        packet.encode(&mut buf).unwrap();

        let mut decode_buf = Cursor::new(buf);
        let decoded_packet = ConnectPacket::decode(&mut decode_buf).unwrap();

        assert_eq!(packet, decoded_packet);
    }
}
