use std::io::{Read, Write};
use std::convert::Into;
use std::ops::Deref;
use std::fmt;
use std::error::Error;

use regex::Regex;

use encodable::StringEncodeError;
use {Encodable, Decodable};

const TOPIC_NAME_VALIDATE_REGEX: &'static str = r"^(\$?[^/\$]+)?(/[^/\$]+)*$";

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct TopicName(String);

impl TopicName {
    pub fn new(topic_name: String) -> Result<TopicName, TopicNameError> {
        let re = Regex::new(TOPIC_NAME_VALIDATE_REGEX).unwrap();
        if topic_name.is_empty() || topic_name.as_bytes().len() > 65535 ||
           !re.is_match(&topic_name[..]) {
            Err(TopicNameError::InvalidTopicName(topic_name))
        } else {
            Ok(TopicName(topic_name))
        }
    }

    pub unsafe fn new_unchecked(topic_name: String) -> TopicName {
        TopicName(topic_name)
    }

    pub fn is_server_specific(&self) -> bool {
        self.0.starts_with('$')
    }
}

impl Into<String> for TopicName {
    fn into(self) -> String {
        self.0
    }
}

impl Deref for TopicName {
    type Target = String;

    fn deref(&self) -> &String {
        &self.0
    }
}

pub struct TopicNameRef<'a>(&'a str);

impl<'a> TopicNameRef<'a> {
    pub fn is_server_specific(&self) -> bool {
        self.0.starts_with('$')
    }
}

impl<'a> Encodable<'a> for TopicName {
    type Err = TopicNameError;

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), TopicNameError> {
        (&self.0[..]).encode(writer).map_err(TopicNameError::StringEncodeError)
    }

    fn encoded_length(&self) -> u32 {
        (&self.0[..]).encoded_length()
    }
}

impl<'a> Decodable<'a> for TopicName {
    type Err = TopicNameError;
    type Cond = ();

    fn decode_with<R: Read>(reader: &mut R,
                            _rest: Option<()>)
                            -> Result<TopicName, TopicNameError> {
        TopicName::new(try!(Decodable::decode(reader).map_err(TopicNameError::StringEncodeError)))
    }
}

#[derive(Debug)]
pub enum TopicNameError {
    StringEncodeError(StringEncodeError),
    InvalidTopicName(String),
}

impl fmt::Display for TopicNameError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &TopicNameError::StringEncodeError(ref err) => err.fmt(f),
            &TopicNameError::InvalidTopicName(ref topic) =>
                write!(f, "Invalid topic filter ({})", topic),
        }
    }
}

impl Error for TopicNameError {
    fn description(&self) -> &str {
        match self {
            &TopicNameError::StringEncodeError(ref err) => err.description(),
            &TopicNameError::InvalidTopicName(..) => "Invalid topic filter",
        }
    }

    fn cause(&self) -> Option<&Error> {
        match self {
            &TopicNameError::StringEncodeError(ref err) => Some(err),
            &TopicNameError::InvalidTopicName(..) => None,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_topic_name_basic() {
        let topic_name = "$SYS".to_owned();
        TopicName::new(topic_name).unwrap();

        let topic_name = "$SYS/broker/connection/test.cosm-energy/state".to_owned();
        TopicName::new(topic_name).unwrap();
    }
}
