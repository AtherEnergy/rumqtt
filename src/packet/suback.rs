use std::io::{self, Read, Write};
use std::error::Error;
use std::fmt;
use std::convert::From;

use byteorder::{self, WriteBytesExt, ReadBytesExt};

use control::{FixedHeader, PacketType, ControlType};
use control::variable_header::PacketIdentifier;
use packet::{Packet, PacketError};
use {Encodable, Decodable};

#[repr(u8)]
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum SubscribeReturnCode {
    MaximumQoSLevel0 = 0x00,
    MaximumQoSLevel1 = 0x01,
    MaximumQoSLevel2 = 0x02,
    Failure = 0x80,
}

#[derive(Debug, Eq, PartialEq)]
pub struct SubackPacket {
    fixed_header: FixedHeader,
    packet_identifier: PacketIdentifier,
    payload: SubackPacketPayload,
}

impl SubackPacket {
    pub fn new(pkid: u16, subscribes: Vec<SubscribeReturnCode>) -> SubackPacket {
        let mut pk = SubackPacket {
            fixed_header: FixedHeader::new(PacketType::with_default(ControlType::SubscribeAcknowledgement), 0),
            packet_identifier: PacketIdentifier(pkid),
            payload: SubackPacketPayload::new(subscribes),
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

impl<'a> Packet<'a> for SubackPacket {
    type Payload = SubackPacketPayload;

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
        let payload: SubackPacketPayload =
            try!(SubackPacketPayload::decode_with(reader,
                                                  Some(fixed_header.remaining_length -
                                                       packet_identifier.encoded_length()))
                     .map_err(PacketError::PayloadError));
        Ok(SubackPacket {
            fixed_header: fixed_header,
            packet_identifier: packet_identifier,
            payload: payload,
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct SubackPacketPayload {
    subscribes: Vec<SubscribeReturnCode>,
}

impl SubackPacketPayload {
    pub fn new(subs: Vec<SubscribeReturnCode>) -> SubackPacketPayload {
        SubackPacketPayload { subscribes: subs }
    }

    pub fn subscribes(&self) -> &[SubscribeReturnCode] {
        &self.subscribes[..]
    }
}

impl<'a> Encodable<'a> for SubackPacketPayload {
    type Err = SubackPacketPayloadError;

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), Self::Err> {
        for code in self.subscribes.iter() {
            try!(writer.write_u8(*code as u8));
        }

        Ok(())
    }

    fn encoded_length(&self) -> u32 {
        self.subscribes.len() as u32
    }
}

impl<'a> Decodable<'a> for SubackPacketPayload {
    type Err = SubackPacketPayloadError;
    type Cond = u32;

    fn decode_with<R: Read>(reader: &mut R,
                            payload_len: Option<u32>)
                            -> Result<SubackPacketPayload, SubackPacketPayloadError> {
        let payload_len = payload_len.expect("Must provide payload length");
        let mut subs = Vec::new();

        for _ in 0..payload_len {
            let retcode = match try!(reader.read_u8()) {
                0x00 => SubscribeReturnCode::MaximumQoSLevel0,
                0x01 => SubscribeReturnCode::MaximumQoSLevel1,
                0x02 => SubscribeReturnCode::MaximumQoSLevel2,
                0x80 => SubscribeReturnCode::Failure,
                code => return Err(SubackPacketPayloadError::InvalidSubscribeReturnCode(code)),
            };

            subs.push(retcode);
        }

        Ok(SubackPacketPayload::new(subs))
    }
}

#[derive(Debug)]
pub enum SubackPacketPayloadError {
    IoError(io::Error),
    InvalidSubscribeReturnCode(u8),
}

impl fmt::Display for SubackPacketPayloadError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &SubackPacketPayloadError::IoError(ref err) => err.fmt(f),
            &SubackPacketPayloadError::InvalidSubscribeReturnCode(code) =>
                write!(f, "Invalid subscribe return code {}", code),
        }
    }
}

impl Error for SubackPacketPayloadError {
    fn description(&self) -> &str {
        match self {
            &SubackPacketPayloadError::IoError(ref err) => err.description(),
            &SubackPacketPayloadError::InvalidSubscribeReturnCode(..) =>
                "Invalid subscribe return code",
        }
    }

    fn cause(&self) -> Option<&Error> {
        match self {
            &SubackPacketPayloadError::IoError(ref err) => Some(err),
            &SubackPacketPayloadError::InvalidSubscribeReturnCode(..) => None,
        }
    }
}

impl From<byteorder::Error> for SubackPacketPayloadError {
    fn from(err: byteorder::Error) -> SubackPacketPayloadError {
        SubackPacketPayloadError::IoError(From::from(err))
    }
}
