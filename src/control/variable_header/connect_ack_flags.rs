use std::io::{Read, Write};
use std::convert::From;

use byteorder::{ReadBytesExt, WriteBytesExt};

use control::variable_header::VariableHeaderError;
use {Encodable, Decodable};

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub struct ConnackFlags {
    pub session_present: bool,
}

impl ConnackFlags {
    pub fn empty() -> ConnackFlags {
        ConnackFlags { session_present: false }
    }
}

impl<'a> Encodable<'a> for ConnackFlags {
    type Err = VariableHeaderError;

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), VariableHeaderError> {
        let code = self.session_present as u8;
        writer.write_u8(code).map_err(From::from)
    }

    fn encoded_length(&self) -> u32 {
        1
    }
}

impl<'a> Decodable<'a> for ConnackFlags {
    type Err = VariableHeaderError;
    type Cond = ();

    fn decode_with<R: Read>(reader: &mut R,
                            _rest: Option<()>)
                            -> Result<ConnackFlags, VariableHeaderError> {
        let code = try!(reader.read_u8());
        if code & !1 != 0 {
            return Err(VariableHeaderError::InvalidReservedFlag);
        }

        Ok(ConnackFlags { session_present: code == 1 })
    }
}
