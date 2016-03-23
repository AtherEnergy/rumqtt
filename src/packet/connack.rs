use std::io::{Read, Write};


use control::{FixedHeader, PacketType, ControlType};
use control::variable_header::{ConnackFlags, ConnectReturnCode};
use packet::{Packet, PacketError};
use {Encodable, Decodable};

#[derive(Debug, Eq, PartialEq)]
pub struct ConnackPacket {
    fixed_header: FixedHeader,
    flags: ConnackFlags,
    ret_code: ConnectReturnCode,
    payload: (),
}

impl ConnackPacket {
    pub fn new(session_present: bool, ret_code: ConnectReturnCode) -> ConnackPacket {
        ConnackPacket {
            fixed_header: FixedHeader::new(PacketType::with_default(ControlType::ConnectAcknowledgement), 2),
            flags: ConnackFlags { session_present: session_present },
            ret_code: ret_code,
            payload: (),
        }
    }

    pub fn connack_flags(&self) -> ConnackFlags {
        self.flags
    }

    pub fn connect_return_code(&self) -> ConnectReturnCode {
        self.ret_code
    }
}

impl<'a> Packet<'a> for ConnackPacket {
    type Payload = ();

    fn fixed_header(&self) -> &FixedHeader {
        &self.fixed_header
    }

    fn payload(&self) -> &Self::Payload {
        &self.payload
    }

    fn encode_variable_headers<W: Write>(&self,
                                         writer: &mut W)
                                         -> Result<(), PacketError<'a, Self>> {
        try!(self.flags.encode(writer));
        try!(self.ret_code.encode(writer));
        Ok(())
    }

    fn encoded_variable_headers_length(&self) -> u32 {
        self.flags.encoded_length() + self.ret_code.encoded_length()
    }

    fn decode_packet<R: Read>(reader: &mut R,
                              fixed_header: FixedHeader)
                              -> Result<Self, PacketError<'a, Self>> {
        let flags: ConnackFlags = try!(Decodable::decode(reader));
        let code: ConnectReturnCode = try!(Decodable::decode(reader));

        Ok(ConnackPacket {
            fixed_header: fixed_header,
            flags: flags,
            ret_code: code,
            payload: (),
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use std::io::Cursor;

    use control::variable_header::ConnectReturnCode;
    use {Encodable, Decodable};

    #[test]
    pub fn test_connack_packet_basic() {
        let packet = ConnackPacket::new(false, ConnectReturnCode::IdentifierRejected);

        let mut buf = Vec::new();
        packet.encode(&mut buf).unwrap();

        let mut decode_buf = Cursor::new(buf);
        let decoded = ConnackPacket::decode(&mut decode_buf).unwrap();

        assert_eq!(packet, decoded);
    }
}
