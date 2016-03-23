use std::io::{Read, Write};


use control::{FixedHeader, PacketType, ControlType};
use packet::{Packet, PacketError};

#[derive(Debug, Eq, PartialEq)]
pub struct PingrespPacket {
    fixed_header: FixedHeader,
    payload: (),
}

impl PingrespPacket {
    pub fn new() -> PingrespPacket {
        PingrespPacket {
            fixed_header: FixedHeader::new(PacketType::with_default(ControlType::PingResponse), 0),
            payload: (),
        }
    }
}

impl<'a> Packet<'a> for PingrespPacket {
    type Payload = ();

    fn fixed_header(&self) -> &FixedHeader {
        &self.fixed_header
    }

    fn payload(&self) -> &Self::Payload {
        &self.payload
    }

    fn encode_variable_headers<W: Write>(&self,
                                         _writer: &mut W)
                                         -> Result<(), PacketError<'a, Self>> {
        Ok(())
    }

    fn encoded_variable_headers_length(&self) -> u32 {
        0
    }

    fn decode_packet<R: Read>(_reader: &mut R,
                              fixed_header: FixedHeader)
                              -> Result<Self, PacketError<'a, Self>> {
        Ok(PingrespPacket {
            fixed_header: fixed_header,
            payload: (),
        })
    }
}
