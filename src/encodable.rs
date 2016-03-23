use std::io::{self, Read, Write};
use std::error::Error;
use std::string::FromUtf8Error;
use std::fmt;
use std::convert::From;
use std::marker::Sized;

use byteorder::{self, BigEndian, WriteBytesExt, ReadBytesExt};

pub trait Encodable<'a> {
    type Err: Error + 'a;

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), Self::Err>;
    fn encoded_length(&self) -> u32;
}

pub trait Decodable<'a>: Sized {
    type Err: Error + 'a;
    type Cond;

    fn decode<R: Read>(reader: &mut R) -> Result<Self, Self::Err> {
        Self::decode_with(reader, None)
    }

    fn decode_with<R: Read>(reader: &mut R, Cond: Option<Self::Cond>) -> Result<Self, Self::Err>;
}

impl<'a> Encodable<'a> for &'a str {
    type Err = StringEncodeError;

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), StringEncodeError> {
        writer.write_u16::<BigEndian>(self.as_bytes().len() as u16)
              .map_err(From::from)
              .and_then(|_| writer.write_all(self.as_bytes()))
              .map_err(StringEncodeError::IoError)
    }

    fn encoded_length(&self) -> u32 {
        2 + self.as_bytes().len() as u32
    }
}

impl<'a> Encodable<'a> for &'a [u8] {
    type Err = io::Error;

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), io::Error> {
        writer.write_all(self)
    }

    fn encoded_length(&self) -> u32 {
        self.len() as u32
    }
}

impl<'a> Encodable<'a> for String {
    type Err = StringEncodeError;

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), StringEncodeError> {
        (&self[..]).encode(writer)
    }

    fn encoded_length(&self) -> u32 {
        (&self[..]).encoded_length()
    }
}

impl<'a> Decodable<'a> for String {
    type Err = StringEncodeError;
    type Cond = ();

    fn decode_with<R: Read>(reader: &mut R,
                            _rest: Option<()>)
                            -> Result<String, StringEncodeError> {
        let len = try!(reader.read_u16::<BigEndian>()) as usize;
        let mut buf = Vec::with_capacity(len);
        try!(reader.take(len as u64).read_to_end(&mut buf));

        if buf.len() < len {
            return Err(StringEncodeError::MalformedData);
        }

        String::from_utf8(buf).map_err(StringEncodeError::FromUtf8Error)
    }
}

impl<'a> Encodable<'a> for Vec<u8> {
    type Err = io::Error;

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), io::Error> {
        (&self[..]).encode(writer)
    }

    fn encoded_length(&self) -> u32 {
        (&self[..]).encoded_length()
    }
}

impl<'a> Decodable<'a> for Vec<u8> {
    type Err = io::Error;
    type Cond = u32;

    fn decode_with<R: Read>(reader: &mut R, length: Option<u32>) -> Result<Vec<u8>, io::Error> {
        match length {
            Some(length) => {
                let mut buf = Vec::with_capacity(length as usize);
                try!(reader.take(length as u64).read_to_end(&mut buf));
                Ok(buf)
            }
            None => {
                let mut buf = Vec::new();
                try!(reader.read_to_end(&mut buf));
                Ok(buf)
            }
        }
    }
}

impl<'a> Encodable<'a> for () {
    type Err = NoError;

    fn encode<W: Write>(&self, _: &mut W) -> Result<(), NoError> {
        Ok(())
    }

    fn encoded_length(&self) -> u32 {
        0
    }
}

impl<'a> Decodable<'a> for () {
    type Err = NoError;
    type Cond = ();

    fn decode_with<R: Read>(_: &mut R, _: Option<()>) -> Result<(), NoError> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct NoError;

impl fmt::Display for NoError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "No error")
    }
}

impl Error for NoError {
    fn description(&self) -> &str {
        "No error"
    }
}

#[derive(Debug)]
pub enum StringEncodeError {
    IoError(io::Error),
    FromUtf8Error(FromUtf8Error),
    MalformedData,
}

impl fmt::Display for StringEncodeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &StringEncodeError::IoError(ref err) => err.fmt(f),
            &StringEncodeError::FromUtf8Error(ref err) => err.fmt(f),
            &StringEncodeError::MalformedData => write!(f, "Malformed data"),
        }
    }
}

impl Error for StringEncodeError {
    fn description(&self) -> &str {
        match self {
            &StringEncodeError::IoError(ref err) => err.description(),
            &StringEncodeError::FromUtf8Error(ref err) => err.description(),
            &StringEncodeError::MalformedData => "Malformed data",
        }
    }

    fn cause(&self) -> Option<&Error> {
        match self {
            &StringEncodeError::IoError(ref err) => Some(err),
            &StringEncodeError::FromUtf8Error(ref err) => Some(err),
            &StringEncodeError::MalformedData => None,
        }
    }
}

impl From<io::Error> for StringEncodeError {
    fn from(err: io::Error) -> StringEncodeError {
        StringEncodeError::IoError(err)
    }
}

impl From<byteorder::Error> for StringEncodeError {
    fn from(err: byteorder::Error) -> StringEncodeError {
        StringEncodeError::IoError(From::from(err))
    }
}

impl From<FromUtf8Error> for StringEncodeError {
    fn from(err: FromUtf8Error) -> StringEncodeError {
        StringEncodeError::FromUtf8Error(err)
    }
}
