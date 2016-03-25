use std::io::{self, Read, Write};
use std::string::FromUtf8Error;
use std::error::Error;
use std::fmt;
use std::convert::From;

use byteorder::{self, WriteBytesExt, ReadBytesExt};

use control::{FixedHeader, PacketType, ControlType};
use control::variable_header::PacketIdentifier;
use packet::{Packet, PacketError};
use {Encodable, Decodable, QualityOfService};
use encodable::StringEncodeError;
use topic_filter::{TopicFilter, TopicFilterError};

#[derive(Debug, Eq, PartialEq)]
pub struct SubscribePacket {
    fixed_header: FixedHeader,
    packet_identifier: PacketIdentifier,
    payload: SubscribePacketPayload,
}

impl SubscribePacket {
    pub fn new(pkid: u16, subscribes: Vec<(TopicFilter, QualityOfService)>) -> SubscribePacket {
        let mut pk = SubscribePacket {
            fixed_header: FixedHeader::new(PacketType::with_default(ControlType::Subscribe), 0),
            packet_identifier: PacketIdentifier(pkid),
            payload: SubscribePacketPayload::new(subscribes),
        };
        pk.fixed_header.remaining_length = pk.encoded_variable_headers_length() +
                                           pk.payload.encoded_length();
        pk
    }

    pub fn packet_identifier(&self) -> u16 {
        self.packet_identifier.0
    }

    pub fn set_packet_identifier(&mut self, pkid: u16) {
        self.packet_identifier.0 = pkid;
    }
}

impl<'a> Packet<'a> for SubscribePacket {
    type Payload = SubscribePacketPayload;

    fn fixed_header(&self) -> &FixedHeader {
        &self.fixed_header
    }

    fn payload(&self) -> &Self::Payload {
        &self.payload
    }

    fn encode_variable_headers<W: Write>(&self,
                                         writer: &mut W)
                                         -> Result<(), PacketError<'a, Self>> {
        try!(self.packet_identifier.encode(writer));

        Ok(())
    }

    fn encoded_variable_headers_length(&self) -> u32 {
        self.packet_identifier.encoded_length()
    }

    fn decode_packet<R: Read>(reader: &mut R,
                              fixed_header: FixedHeader)
                              -> Result<Self, PacketError<'a, Self>> {
        let packet_identifier: PacketIdentifier = try!(PacketIdentifier::decode(reader));
        let payload: SubscribePacketPayload = try!(SubscribePacketPayload::decode_with(reader,
                                                     Some(fixed_header.remaining_length -
                                                          packet_identifier.encoded_length()))
                     .map_err(PacketError::PayloadError));
        Ok(SubscribePacket {
            fixed_header: fixed_header,
            packet_identifier: packet_identifier,
            payload: payload,
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct SubscribePacketPayload {
    subscribes: Vec<(TopicFilter, QualityOfService)>,
}

impl SubscribePacketPayload {
    pub fn new(subs: Vec<(TopicFilter, QualityOfService)>) -> SubscribePacketPayload {
        SubscribePacketPayload { subscribes: subs }
    }

    pub fn subscribes(&self) -> &[(TopicFilter, QualityOfService)] {
        &self.subscribes[..]
    }
}

impl<'a> Encodable<'a> for SubscribePacketPayload {
    type Err = SubscribePacketPayloadError;

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), Self::Err> {
        for &(ref filter, ref qos) in self.subscribes.iter() {
            try!(filter.encode(writer));
            try!(writer.write_u8(*qos as u8));
        }

        Ok(())
    }

    fn encoded_length(&self) -> u32 {
        self.subscribes
            .iter()
            .fold(0, |b, a| b + a.0.encoded_length() + 1)
    }
}

impl<'a> Decodable<'a> for SubscribePacketPayload {
    type Err = SubscribePacketPayloadError;
    type Cond = u32;

    fn decode_with<R: Read>(reader: &mut R,
                            payload_len: Option<u32>)
                            -> Result<SubscribePacketPayload, SubscribePacketPayloadError> {
        let mut payload_len = payload_len.expect("Must provide payload length");
        let mut subs = Vec::new();

        while payload_len > 0 {
            let filter = try!(TopicFilter::decode(reader));
            let qos = match try!(reader.read_u8()) {
                0 => QualityOfService::Level0,
                1 => QualityOfService::Level1,
                2 => QualityOfService::Level2,
                _ => return Err(SubscribePacketPayloadError::InvalidQualityOfService),
            };

            payload_len -= filter.encoded_length() + 1;
            subs.push((filter, qos));
        }

        Ok(SubscribePacketPayload::new(subs))
    }
}

#[derive(Debug)]
pub enum SubscribePacketPayloadError {
    IoError(io::Error),
    FromUtf8Error(FromUtf8Error),
    StringEncodeError(StringEncodeError),
    InvalidQualityOfService,
    TopicFilterError(TopicFilterError),
}

impl fmt::Display for SubscribePacketPayloadError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &SubscribePacketPayloadError::IoError(ref err) => err.fmt(f),
            &SubscribePacketPayloadError::FromUtf8Error(ref err) => err.fmt(f),
            &SubscribePacketPayloadError::StringEncodeError(ref err) => err.fmt(f),
            &SubscribePacketPayloadError::InvalidQualityOfService =>
                write!(f, "Invalid quality of service"),
            &SubscribePacketPayloadError::TopicFilterError(ref err) => err.fmt(f),
        }
    }
}

impl Error for SubscribePacketPayloadError {
    fn description(&self) -> &str {
        match self {
            &SubscribePacketPayloadError::IoError(ref err) => err.description(),
            &SubscribePacketPayloadError::FromUtf8Error(ref err) => err.description(),
            &SubscribePacketPayloadError::StringEncodeError(ref err) => err.description(),
            &SubscribePacketPayloadError::InvalidQualityOfService => "Invalid quality of service",
            &SubscribePacketPayloadError::TopicFilterError(ref err) => err.description(),
        }
    }

    fn cause(&self) -> Option<&Error> {
        match self {
            &SubscribePacketPayloadError::IoError(ref err) => Some(err),
            &SubscribePacketPayloadError::FromUtf8Error(ref err) => Some(err),
            &SubscribePacketPayloadError::StringEncodeError(ref err) => Some(err),
            &SubscribePacketPayloadError::InvalidQualityOfService => None,
            &SubscribePacketPayloadError::TopicFilterError(ref err) => Some(err),
        }
    }
}

impl From<TopicFilterError> for SubscribePacketPayloadError {
    fn from(err: TopicFilterError) -> SubscribePacketPayloadError {
        SubscribePacketPayloadError::TopicFilterError(err)
    }
}

impl From<StringEncodeError> for SubscribePacketPayloadError {
    fn from(err: StringEncodeError) -> SubscribePacketPayloadError {
        SubscribePacketPayloadError::StringEncodeError(err)
    }
}

impl From<byteorder::Error> for SubscribePacketPayloadError {
    fn from(err: byteorder::Error) -> SubscribePacketPayloadError {
        SubscribePacketPayloadError::IoError(From::from(err))
    }
}
