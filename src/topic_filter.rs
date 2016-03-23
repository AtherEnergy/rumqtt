use std::io::{Read, Write};
use std::fmt;
use std::error::Error;
use std::ops::Deref;
use std::mem;
use std::convert::Into;

use regex::Regex;

use {Encodable, Decodable};
use encodable::StringEncodeError;

const VALIDATE_TOPIC_FILTER_REGEX: &'static str =
    r"^(#|((\+|\$?[^/\$\+#]+)?(/(\+|[^/\$\+#]+))*?(/(\+|#|[^/\$\+#]+))?))$";

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct TopicFilter(String);

impl TopicFilter {
    pub fn new_checked<S: Into<String>>(topic: S) -> Result<TopicFilter, TopicFilterError> {
        let topic = topic.into();
        let re = Regex::new(VALIDATE_TOPIC_FILTER_REGEX).unwrap();
        if topic.is_empty() || topic.as_bytes().len() > 65535 || !re.is_match(&topic[..]) {
            Err(TopicFilterError::InvalidTopicFilter(topic))
        } else {
            Ok(TopicFilter(topic))
        }
    }

    pub fn new<S: Into<String>>(topic: S) -> TopicFilter {
        TopicFilter(topic.into())
    }
}

impl<'a> Encodable<'a> for TopicFilter {
    type Err = TopicFilterError;

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), TopicFilterError> {
        (&self.0[..]).encode(writer).map_err(TopicFilterError::StringEncodeError)
    }

    fn encoded_length(&self) -> u32 {
        (&self.0[..]).encoded_length()
    }
}

impl<'a> Decodable<'a> for TopicFilter {
    type Err = TopicFilterError;
    type Cond = ();

    fn decode_with<R: Read>(reader: &mut R,
                            _rest: Option<()>)
                            -> Result<TopicFilter, TopicFilterError> {
        let topic_filter: String = try!(Decodable::decode(reader)
                                            .map_err(TopicFilterError::StringEncodeError));
        TopicFilter::new_checked(topic_filter)
    }
}

impl Deref for TopicFilter {
    type Target = TopicFilterRef;

    fn deref(&self) -> &TopicFilterRef {
        TopicFilterRef::new(&self.0)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct TopicFilterRef(str);

impl TopicFilterRef {
    pub fn new_checked<S: AsRef<str> + ?Sized>(topic: &S)
                                               -> Result<&TopicFilterRef, TopicFilterError> {
        let re = Regex::new(VALIDATE_TOPIC_FILTER_REGEX).unwrap();
        let topic = topic.as_ref();
        if topic.is_empty() || topic.as_bytes().len() > 65535 || !re.is_match(&topic[..]) {
            Err(TopicFilterError::InvalidTopicFilter(topic.to_owned()))
        } else {
            Ok(unsafe { mem::transmute(topic) })
        }
    }

    pub fn new<S: AsRef<str> + ?Sized>(topic: &S) -> &TopicFilterRef {
        unsafe { mem::transmute(topic.as_ref()) }
    }
}

impl Deref for TopicFilterRef {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

#[derive(Debug)]
pub enum TopicFilterError {
    StringEncodeError(StringEncodeError),
    InvalidTopicFilter(String),
}

impl fmt::Display for TopicFilterError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &TopicFilterError::StringEncodeError(ref err) => err.fmt(f),
            &TopicFilterError::InvalidTopicFilter(ref topic) =>
                write!(f, "Invalid topic filter ({})", topic),
        }
    }
}

impl Error for TopicFilterError {
    fn description(&self) -> &str {
        match self {
            &TopicFilterError::StringEncodeError(ref err) => err.description(),
            &TopicFilterError::InvalidTopicFilter(..) => "Invalid topic filter",
        }
    }

    fn cause(&self) -> Option<&Error> {
        match self {
            &TopicFilterError::StringEncodeError(ref err) => Some(err),
            &TopicFilterError::InvalidTopicFilter(..) => None,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_topic_filter_validate() {
        let topic = "#".to_owned();
        TopicFilter::new_checked(topic).unwrap();

        let topic = "sport/tennis/player1".to_owned();
        TopicFilter::new_checked(topic).unwrap();

        let topic = "sport/tennis/player1/ranking".to_owned();
        TopicFilter::new_checked(topic).unwrap();

        let topic = "sport/tennis/player1/#".to_owned();
        TopicFilter::new_checked(topic).unwrap();

        let topic = "#".to_owned();
        TopicFilter::new_checked(topic).unwrap();

        let topic = "sport/tennis/#".to_owned();
        TopicFilter::new_checked(topic).unwrap();

        let topic = "sport/tennis#".to_owned();
        assert!(TopicFilter::new_checked(topic).is_err());

        let topic = "sport/tennis/#/ranking".to_owned();
        assert!(TopicFilter::new_checked(topic).is_err());

        let topic = "+".to_owned();
        TopicFilter::new_checked(topic).unwrap();

        let topic = "+/tennis/#".to_owned();
        TopicFilter::new_checked(topic).unwrap();

        let topic = "sport+".to_owned();
        assert!(TopicFilter::new_checked(topic).is_err());

        let topic = "sport/+/player1".to_owned();
        TopicFilter::new_checked(topic).unwrap();

        let topic = "+/+".to_owned();
        TopicFilter::new_checked(topic).unwrap();

        let topic = "$SYS/#".to_owned();
        TopicFilter::new_checked(topic).unwrap();

        let topic = "$SYS".to_owned();
        TopicFilter::new_checked(topic).unwrap();
    }
}
