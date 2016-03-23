use std::io::{Read, Write};
use std::convert::From;

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};

use control::variable_header::VariableHeaderError;
use {Encodable, Decodable};

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub struct KeepAlive(pub u16);

impl<'a> Encodable<'a> for KeepAlive {
    type Err = VariableHeaderError;

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), VariableHeaderError> {
        writer.write_u16::<BigEndian>(self.0)
              .map_err(From::from)
    }

    fn encoded_length(&self) -> u32 {
        2
    }
}

impl<'a> Decodable<'a> for KeepAlive {
    type Err = VariableHeaderError;
    type Cond = ();

    fn decode_with<R: Read>(reader: &mut R,
                            _rest: Option<()>)
                            -> Result<KeepAlive, VariableHeaderError> {
        reader.read_u16::<BigEndian>()
              .map(KeepAlive)
              .map_err(From::from)
    }
}
