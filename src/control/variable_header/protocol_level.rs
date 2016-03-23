use std::io::{Read, Write};
use std::convert::From;

use byteorder::{ReadBytesExt, WriteBytesExt};

use control::variable_header::VariableHeaderError;
use {Encodable, Decodable};

pub const SPEC_3_1_1: u8 = 0x04;

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub struct ProtocolLevel(pub u8);

impl<'a> Encodable<'a> for ProtocolLevel {
    type Err = VariableHeaderError;

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), VariableHeaderError> {
        writer.write_u8(self.0)
              .map_err(From::from)
    }

    fn encoded_length(&self) -> u32 {
        1
    }
}

impl<'a> Decodable<'a> for ProtocolLevel {
    type Err = VariableHeaderError;
    type Cond = ();

    fn decode_with<R: Read>(reader: &mut R,
                            _rest: Option<()>)
                            -> Result<ProtocolLevel, VariableHeaderError> {
        reader.read_u8()
              .map(ProtocolLevel)
              .map_err(From::from)
    }
}
