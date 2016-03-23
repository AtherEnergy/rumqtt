use std::io::{self, Read, Write};
use std::error::Error;
use std::fmt;
use std::convert::From;

use control::FixedHeader;
use control::fixed_header::FixedHeaderError;
use control::variable_header::VariableHeaderError;
use control::ControlType;
use encodable::StringEncodeError;
use topic_name::TopicNameError;
use {Encodable, Decodable};

pub use self::connect::ConnectPacket;
pub use self::connack::ConnackPacket;
pub use self::publish::PublishPacket;
pub use self::puback::PubackPacket;
pub use self::pubrec::PubrecPacket;
pub use self::pubrel::PubrelPacket;
pub use self::pubcomp::PubcompPacket;
pub use self::pingreq::PingreqPacket;
pub use self::pingresp::PingrespPacket;
pub use self::disconnect::DisconnectPacket;
pub use self::subscribe::SubscribePacket;
pub use self::suback::SubackPacket;
pub use self::unsuback::UnsubackPacket;
pub use self::unsubscribe::UnsubscribePacket;

pub use self::publish::QoSWithPacketIdentifier;

pub mod connect;
pub mod connack;
pub mod publish;
pub mod puback;
pub mod pubrec;
pub mod pubrel;
pub mod pubcomp;
pub mod pingreq;
pub mod pingresp;
pub mod disconnect;
pub mod subscribe;
pub mod suback;
pub mod unsuback;
pub mod unsubscribe;

pub trait Packet<'a>: Sized {
    type Payload: Encodable<'a> + Decodable<'a> + 'a;

    fn fixed_header(&self) -> &FixedHeader;
    fn payload(&self) -> &Self::Payload;

    fn encode_variable_headers<W: Write>(&self,
                                         writer: &mut W)
                                         -> Result<(), PacketError<'a, Self>>;
    fn encoded_variable_headers_length(&self) -> u32;
    fn decode_packet<R: Read>(reader: &mut R,
                              fixed_header: FixedHeader)
                              -> Result<Self, PacketError<'a, Self>>;
}

impl<'a, T: Packet<'a> + fmt::Debug + 'a> Encodable<'a> for T {
    type Err = PacketError<'a, T>;

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), PacketError<'a, T>> {
        try!(self.fixed_header().encode(writer));
        try!(self.encode_variable_headers(writer));

        self.payload().encode(writer).map_err(PacketError::PayloadError)
    }

    fn encoded_length(&self) -> u32 {
        self.fixed_header().encoded_length() + self.encoded_variable_headers_length() +
        self.payload().encoded_length()
    }
}

impl<'a, T: Packet<'a> + fmt::Debug + 'a> Decodable<'a> for T {
    type Err = PacketError<'a, T>;
    type Cond = FixedHeader;

    fn decode_with<R: Read>(reader: &mut R,
                            fixed_header: Option<FixedHeader>)
                            -> Result<Self, PacketError<'a, Self>> {
        let fixed_header: FixedHeader = if let Some(hdr) = fixed_header {
            hdr
        } else {
            try!(Decodable::decode(reader))
        };

        <Self as Packet>::decode_packet(reader, fixed_header)
    }
}

#[derive(Debug)]
pub enum PacketError<'a, T: Packet<'a>> {
    FixedHeaderError(FixedHeaderError),
    VariableHeaderError(VariableHeaderError),
    PayloadError(<<T as Packet<'a>>::Payload as Encodable<'a>>::Err),
    MalformedPacket(String),
    StringEncodeError(StringEncodeError),
    IoError(io::Error),
    TopicNameError(TopicNameError),
}

impl<'a, T: Packet<'a>> fmt::Display for PacketError<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &PacketError::FixedHeaderError(ref err) => err.fmt(f),
            &PacketError::VariableHeaderError(ref err) => err.fmt(f),
            &PacketError::PayloadError(ref err) => err.fmt(f),
            &PacketError::MalformedPacket(ref err) => err.fmt(f),
            &PacketError::StringEncodeError(ref err) => err.fmt(f),
            &PacketError::IoError(ref err) => err.fmt(f),
            &PacketError::TopicNameError(ref err) => err.fmt(f),
        }
    }
}

impl<'a, T: Packet<'a> + fmt::Debug> Error for PacketError<'a, T> {
    fn description(&self) -> &str {
        match self {
            &PacketError::FixedHeaderError(ref err) => err.description(),
            &PacketError::VariableHeaderError(ref err) => err.description(),
            &PacketError::PayloadError(ref err) => err.description(),
            &PacketError::MalformedPacket(ref err) => &err[..],
            &PacketError::StringEncodeError(ref err) => err.description(),
            &PacketError::IoError(ref err) => err.description(),
            &PacketError::TopicNameError(ref err) => err.description(),
        }
    }

    fn cause(&self) -> Option<&Error> {
        match self {
            &PacketError::FixedHeaderError(ref err) => Some(err),
            &PacketError::VariableHeaderError(ref err) => Some(err),
            &PacketError::PayloadError(ref err) => Some(err),
            &PacketError::MalformedPacket(..) => None,
            &PacketError::StringEncodeError(ref err) => Some(err),
            &PacketError::IoError(ref err) => Some(err),
            &PacketError::TopicNameError(ref err) => Some(err),
        }
    }
}

impl<'a, T: Packet<'a>> From<FixedHeaderError> for PacketError<'a, T> {
    fn from(err: FixedHeaderError) -> PacketError<'a, T> {
        PacketError::FixedHeaderError(err)
    }
}

impl<'a, T: Packet<'a>> From<VariableHeaderError> for PacketError<'a, T> {
    fn from(err: VariableHeaderError) -> PacketError<'a, T> {
        PacketError::VariableHeaderError(err)
    }
}

impl<'a, T: Packet<'a>> From<io::Error> for PacketError<'a, T> {
    fn from(err: io::Error) -> PacketError<'a, T> {
        PacketError::IoError(err)
    }
}

impl<'a, T: Packet<'a>> From<StringEncodeError> for PacketError<'a, T> {
    fn from(err: StringEncodeError) -> PacketError<'a, T> {
        PacketError::StringEncodeError(err)
    }
}

impl<'a, T: Packet<'a>> From<TopicNameError> for PacketError<'a, T> {
    fn from(err: TopicNameError) -> PacketError<'a, T> {
        PacketError::TopicNameError(err)
    }
}

macro_rules! impl_variable_packet {
    ($($name:ident & $errname:ident => $hdr:ident,)+) => {
        #[derive(Debug, Eq, PartialEq)]
        pub enum VariablePacket {
            $(
                $name($name),
            )+
        }

        $(
            impl From<$name> for VariablePacket {
                fn from(pk: $name) -> VariablePacket {
                    VariablePacket::$name(pk)
                }
            }
        )+

        impl<'a> Encodable<'a> for VariablePacket {
            type Err = VariablePacketError<'a>;

            fn encode<W: Write>(&self, writer: &mut W) -> Result<(), VariablePacketError<'a>> {
                match self {
                    $(
                        &VariablePacket::$name(ref pk) => pk.encode(writer).map_err(From::from),
                    )+
                }
            }

            fn encoded_length(&self) -> u32 {
                match self {
                    $(
                        &VariablePacket::$name(ref pk) => pk.encoded_length(),
                    )+
                }
            }
        }

        impl<'a> Decodable<'a> for VariablePacket {
            type Err = VariablePacketError<'a>;
            type Cond = FixedHeader;

            fn decode_with<R: Read>(reader: &mut R, fixed_header: Option<FixedHeader>)
                    -> Result<VariablePacket, Self::Err> {
                let fixed_header = match fixed_header {
                    Some(fh) => fh,
                    None => {
                        match FixedHeader::decode(reader) {
                            Ok(header) => header,
                            Err(FixedHeaderError::Unrecognized(code, length)) => {
                                let reader = &mut reader.take(length as u64);
                                let mut buf = Vec::with_capacity(length as usize);
                                try!(reader.read_to_end(&mut buf));
                                return Err(VariablePacketError::UnrecognizedPacket(code, buf));
                            },
                            Err(FixedHeaderError::ReservedType(code, length)) => {
                                let reader = &mut reader.take(length as u64);
                                let mut buf = Vec::with_capacity(length as usize);
                                try!(reader.read_to_end(&mut buf));
                                return Err(VariablePacketError::ReservedPacket(code, buf));
                            },
                            Err(err) => return Err(From::from(err))
                        }
                    }
                };
                let reader = &mut reader.take(fixed_header.remaining_length as u64);

                match fixed_header.packet_type.control_type {
                    $(
                        ControlType::$hdr => {
                            let pk = try!(<$name as Packet<'a>>::decode_packet(reader, fixed_header));
                            Ok(VariablePacket::$name(pk))
                        }
                    )+
                }
            }
        }

        #[derive(Debug)]
        pub enum VariablePacketError<'a> {
            FixedHeaderError(FixedHeaderError),
            UnrecognizedPacket(u8, Vec<u8>),
            ReservedPacket(u8, Vec<u8>),
            IoError(io::Error),
            $(
                $errname(PacketError<'a, $name>),
            )+
        }

        impl<'a> From<FixedHeaderError> for VariablePacketError<'a> {
            fn from(err: FixedHeaderError) -> VariablePacketError<'a> {
                VariablePacketError::FixedHeaderError(err)
            }
        }

        impl<'a> From<io::Error> for VariablePacketError<'a> {
            fn from(err: io::Error) -> VariablePacketError<'a> {
                VariablePacketError::IoError(err)
            }
        }

        $(
            impl<'a> From<PacketError<'a, $name>> for VariablePacketError<'a> {
                fn from(err: PacketError<'a, $name>) -> VariablePacketError<'a> {
                    VariablePacketError::$errname(err)
                }
            }
        )+

        impl<'a> fmt::Display for VariablePacketError<'a> {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                match self {
                    &VariablePacketError::FixedHeaderError(ref err) => err.fmt(f),
                    &VariablePacketError::UnrecognizedPacket(ref code, ref v) =>
                        write!(f, "Unrecognized type ({}), [u8, ..{}]", code, v.len()),
                    &VariablePacketError::ReservedPacket(ref code, ref v) =>
                        write!(f, "Reserved type ({}), [u8, ..{}]", code, v.len()),
                    &VariablePacketError::IoError(ref err) => err.fmt(f),
                    $(
                        &VariablePacketError::$errname(ref err) => err.fmt(f),
                    )+
                }
            }
        }

        impl<'a> Error for VariablePacketError<'a> {
            fn description(&self) -> &str {
                match self {
                    &VariablePacketError::FixedHeaderError(ref err) => err.description(),
                    &VariablePacketError::UnrecognizedPacket(..) => "Unrecognized packet",
                    &VariablePacketError::ReservedPacket(..) => "Reserved packet",
                    &VariablePacketError::IoError(ref err) => err.description(),
                    $(
                        &VariablePacketError::$errname(ref err) => err.description(),
                    )+
                }
            }

            fn cause(&self) -> Option<&Error> {
                match self {
                    &VariablePacketError::FixedHeaderError(ref err) => Some(err),
                    &VariablePacketError::UnrecognizedPacket(..) => None,
                    &VariablePacketError::ReservedPacket(..) => None,
                    &VariablePacketError::IoError(ref err) => Some(err),
                    $(
                        &VariablePacketError::$errname(ref err) => Some(err),
                    )+
                }
            }
        }
    }
}

impl_variable_packet! {
    ConnectPacket       & ConnectPacketError        => Connect,
    ConnackPacket       & ConnackPacketError        => ConnectAcknowledgement,

    PublishPacket       & PublishPacketError        => Publish,
    PubackPacket        & PubackPacketError         => PublishAcknowledgement,
    PubrecPacket        & PubrecPacketError         => PublishReceived,
    PubrelPacket        & PubrelPacketError         => PublishRelease,
    PubcompPacket       & PubcompPacketError        => PublishComplete,

    PingreqPacket       & PingreqPacketError        => PingRequest,
    PingrespPacket      & PingrespPacketError       => PingResponse,

    SubscribePacket     & SubscribePacketError      => Subscribe,
    SubackPacket        & SubackPacketError         => SubscribeAcknowledgement,

    UnsubscribePacket   & UnsubscribePacketError    => Unsubscribe,
    UnsubackPacket      & UnsubackPacketError       => UnsubscribeAcknowledgement,

    DisconnectPacket    & DisconnectPacketError     => Disconnect,
}

impl VariablePacket {
    pub fn new<T>(t: T) -> VariablePacket
        where VariablePacket: From<T>
    {
        From::from(t)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use std::io::Cursor;

    use {Encodable, Decodable};

    #[test]
    fn test_variable_packet_basic() {
        let packet = ConnectPacket::new("MQTT".to_owned(), "1234".to_owned());

        // Wrap it
        let var_packet = VariablePacket::new(packet);

        // Encode
        let mut buf = Vec::new();
        var_packet.encode(&mut buf).unwrap();

        // Decode
        let mut decode_buf = Cursor::new(buf);
        let decoded_packet = VariablePacket::decode(&mut decode_buf).unwrap();

        assert_eq!(var_packet, decoded_packet);
    }
}
