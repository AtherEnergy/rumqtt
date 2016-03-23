use std::io::{Read, Write};
use std::convert::From;

use byteorder::{ReadBytesExt, WriteBytesExt};

use control::variable_header::VariableHeaderError;
use {Encodable, Decodable};

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub struct ConnectFlags {
    pub user_name: bool,
    pub password: bool,
    pub will_retain: bool,
    pub will_qos: u8,
    pub will_flag: bool,
    pub clean_session: bool,
}

impl ConnectFlags {
    pub fn empty() -> ConnectFlags {
        ConnectFlags {
            user_name: false,
            password: false,
            will_retain: false,
            will_qos: 0,
            will_flag: false,
            clean_session: false,
        }
    }
}

impl<'a> Encodable<'a> for ConnectFlags {
    type Err = VariableHeaderError;

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), VariableHeaderError> {
        let code = ((self.user_name as u8) << 7) | ((self.password as u8) << 6) |
                   ((self.will_retain as u8) << 5) | ((self.will_qos) << 3) |
                   ((self.will_flag as u8) << 2) |
                   ((self.clean_session as u8) << 1);

        writer.write_u8(code).map_err(From::from)
    }

    fn encoded_length(&self) -> u32 {
        1
    }
}

impl<'a> Decodable<'a> for ConnectFlags {
    type Err = VariableHeaderError;
    type Cond = ();

    fn decode_with<R: Read>(reader: &mut R,
                            _rest: Option<()>)
                            -> Result<ConnectFlags, VariableHeaderError> {
        let code = try!(reader.read_u8());
        if code & 1 != 0 {
            return Err(VariableHeaderError::InvalidReservedFlag);
        }

        Ok(ConnectFlags {
            user_name: (code & 0b1000_0000) != 0,
            password: (code & 0b0100_0000) != 0,
            will_retain: (code & 0b0010_0000) != 0,
            will_qos: (code & 0b0001_1000) >> 3,
            will_flag: (code & 0b0000_0100) != 0,
            clean_session: (code & 0b0000_0010) != 0,
        })
    }
}
