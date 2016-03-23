use std::io::{Read, Write};
use std::convert::{From, Into};
use std::ops::Deref;

use control::variable_header::VariableHeaderError;
use topic_name::TopicName;
use {Encodable, Decodable};

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct TopicNameHeader(TopicName);

impl TopicNameHeader {
    pub fn new(topic_name: String) -> Result<TopicNameHeader, VariableHeaderError> {
        match TopicName::new(topic_name) {
            Ok(h) => Ok(TopicNameHeader(h)),
            Err(err) => Err(VariableHeaderError::TopicNameError(err)),
        }
    }
}

impl Into<TopicName> for TopicNameHeader {
    fn into(self) -> TopicName {
        self.0
    }
}

impl Deref for TopicNameHeader {
    type Target = String;

    fn deref(&self) -> &String {
        &self.0
    }
}

impl<'a> Encodable<'a> for TopicNameHeader {
    type Err = VariableHeaderError;

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), VariableHeaderError> {
        (&self.0[..]).encode(writer).map_err(From::from)
    }

    fn encoded_length(&self) -> u32 {
        (&self.0[..]).encoded_length()
    }
}

impl<'a> Decodable<'a> for TopicNameHeader {
    type Err = VariableHeaderError;
    type Cond = ();

    fn decode_with<R: Read>(reader: &mut R,
                            _rest: Option<()>)
                            -> Result<TopicNameHeader, VariableHeaderError> {
        TopicNameHeader::new(try!(Decodable::decode(reader)))
    }
}
