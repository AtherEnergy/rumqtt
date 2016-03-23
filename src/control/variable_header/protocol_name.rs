use std::io::{Read, Write};
use std::convert::From;

use control::variable_header::VariableHeaderError;
use {Encodable, Decodable};

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct ProtocolName(pub String);

impl<'a> Encodable<'a> for ProtocolName {
    type Err = VariableHeaderError;

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), VariableHeaderError> {
        (&self.0[..]).encode(writer).map_err(From::from)
    }

    fn encoded_length(&self) -> u32 {
        (&self.0[..]).encoded_length()
    }
}

impl<'a> Decodable<'a> for ProtocolName {
    type Err = VariableHeaderError;
    type Cond = ();

    fn decode_with<R: Read>(reader: &mut R,
                            _rest: Option<()>)
                            -> Result<ProtocolName, VariableHeaderError> {
        Ok(ProtocolName(try!(Decodable::decode(reader))))
    }
}
