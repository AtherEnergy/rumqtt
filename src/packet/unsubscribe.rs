use std::io::{self, Read, Write};
use std::string::FromUtf8Error;
use std::error::Error;
use std::fmt;
use std::convert::From;

use byteorder;

use control::{FixedHeader, PacketType, ControlType};
use control::variable_header::PacketIdentifier;
use packet::{Packet, PacketError};
use {Encodable, Decodable};
use encodable::StringEncodeError;

#[derive(Debug, Eq, PartialEq)]
pub struct UnsubscribePacket {
    fixed_header: FixedHeader,
    packet_identifier: PacketIdentifier,
    payload: UnsubscribePacketPayload,
}

impl UnsubscribePacket {
    pub fn new(pkid: u16, subscribes: Vec<String>) -> UnsubscribePacket {
        let mut pk = UnsubscribePacket {
            fixed_header: FixedHeader::new(PacketType::with_default(ControlType::Unsubscribe), 0),
            packet_identifier: PacketIdentifier(pkid),
            payload: UnsubscribePacketPayload::new(subscribes),
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

impl<'a> Packet<'a> for UnsubscribePacket {
    type Payload = UnsubscribePacketPayload;

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
        let payload: UnsubscribePacketPayload =
            try!(UnsubscribePacketPayload::decode_with(reader,
                                                       Some(fixed_header.remaining_length -
                                                            packet_identifier.encoded_length()))
                     .map_err(PacketError::PayloadError));
        Ok(UnsubscribePacket {
            fixed_header: fixed_header,
            packet_identifier: packet_identifier,
            payload: payload,
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct UnsubscribePacketPayload {
    subscribes: Vec<String>,
}

impl UnsubscribePacketPayload {
    pub fn new(subs: Vec<String>) -> UnsubscribePacketPayload {
        UnsubscribePacketPayload { subscribes: subs }
    }

    pub fn subscribes(&self) -> &[String] {
        &self.subscribes[..]
    }
}

impl<'a> Encodable<'a> for UnsubscribePacketPayload {
    type Err = UnsubscribePacketPayloadError;

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), Self::Err> {
        for filter in self.subscribes.iter() {
            try!(filter.encode(writer));
        }

        Ok(())
    }

    fn encoded_length(&self) -> u32 {
        self.subscribes
            .iter()
            .fold(0, |b, a| b + a.encoded_length())
    }
}

impl<'a> Decodable<'a> for UnsubscribePacketPayload {
    type Err = UnsubscribePacketPayloadError;
    type Cond = u32;

    fn decode_with<R: Read>(reader: &mut R,
                            payload_len: Option<u32>)
                            -> Result<UnsubscribePacketPayload, UnsubscribePacketPayloadError> {
        let mut payload_len = payload_len.expect("Must provide payload length");
        let mut subs = Vec::new();

        while payload_len > 0 {
            let filter = try!(String::decode(reader));
            payload_len -= filter.encoded_length();
            subs.push(filter);
        }

        Ok(UnsubscribePacketPayload::new(subs))
    }
}

#[derive(Debug)]
pub enum UnsubscribePacketPayloadError {
    IoError(io::Error),
    FromUtf8Error(FromUtf8Error),
    StringEncodeError(StringEncodeError),
}

impl fmt::Display for UnsubscribePacketPayloadError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &UnsubscribePacketPayloadError::IoError(ref err) => err.fmt(f),
            &UnsubscribePacketPayloadError::FromUtf8Error(ref err) => err.fmt(f),
            &UnsubscribePacketPayloadError::StringEncodeError(ref err) => err.fmt(f),
        }
    }
}

impl Error for UnsubscribePacketPayloadError {
    fn description(&self) -> &str {
        match self {
            &UnsubscribePacketPayloadError::IoError(ref err) => err.description(),
            &UnsubscribePacketPayloadError::FromUtf8Error(ref err) => err.description(),
            &UnsubscribePacketPayloadError::StringEncodeError(ref err) => err.description(),
        }
    }

    fn cause(&self) -> Option<&Error> {
        match self {
            &UnsubscribePacketPayloadError::IoError(ref err) => Some(err),
            &UnsubscribePacketPayloadError::FromUtf8Error(ref err) => Some(err),
            &UnsubscribePacketPayloadError::StringEncodeError(ref err) => Some(err),
        }
    }
}

impl From<StringEncodeError> for UnsubscribePacketPayloadError {
    fn from(err: StringEncodeError) -> UnsubscribePacketPayloadError {
        UnsubscribePacketPayloadError::StringEncodeError(err)
    }
}

impl From<byteorder::Error> for UnsubscribePacketPayloadError {
    fn from(err: byteorder::Error) -> UnsubscribePacketPayloadError {
        UnsubscribePacketPayloadError::IoError(From::from(err))
    }
}
