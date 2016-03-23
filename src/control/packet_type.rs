use std::error::Error;
use std::fmt;

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub struct PacketType {
    pub control_type: ControlType,
    pub flags: u8,
}

#[repr(u8)]
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum ControlType {
    /// Client request to connect to Server
    Connect = value::CONNECT,

    /// Connect acknowledgment
    ConnectAcknowledgement = value::CONNACK,

    /// Publish message
    Publish = value::PUBLISH,

    /// Publish acknowledgment
    PublishAcknowledgement = value::PUBACK,

    /// Publish received (assured delivery part 1)
    PublishReceived = value::PUBREC,

    /// Publish release (assured delivery part 2)
    PublishRelease = value::PUBREL,

    /// Publish complete (assured delivery part 3)
    PublishComplete = value::PUBCOMP,

    /// Client subscribe request
    Subscribe = value::SUBSCRIBE,

    /// Subscribe acknowledgment
    SubscribeAcknowledgement = value::SUBACK,

    /// Unsubscribe request
    Unsubscribe = value::UNSUBSCRIBE,

    /// Unsubscribe acknowledgment
    UnsubscribeAcknowledgement = value::UNSUBACK,

    /// PING request
    PingRequest = value::PINGREQ,

    /// PING response
    PingResponse = value::PINGRESP,

    /// Client is disconnecting
    Disconnect = value::DISCONNECT,
}

impl PacketType {
    #[inline]
    pub fn new(t: ControlType, flags: u8) -> PacketType {
        PacketType {
            control_type: t,
            flags: flags,
        }
    }

    #[inline]
    pub fn with_default(t: ControlType) -> PacketType {
        match t {
            ControlType::Connect => PacketType::new(t, 0),
            ControlType::ConnectAcknowledgement => PacketType::new(t, 0),

            ControlType::Publish => PacketType::new(t, 0),
            ControlType::PublishAcknowledgement => PacketType::new(t, 0),
            ControlType::PublishReceived => PacketType::new(t, 0),
            ControlType::PublishRelease => PacketType::new(t, 0x02),
            ControlType::PublishComplete => PacketType::new(t, 0),

            ControlType::Subscribe => PacketType::new(t, 0x02),
            ControlType::SubscribeAcknowledgement => PacketType::new(t, 0),

            ControlType::Unsubscribe => PacketType::new(t, 0x02),
            ControlType::UnsubscribeAcknowledgement => PacketType::new(t, 0),

            ControlType::PingRequest => PacketType::new(t, 0),
            ControlType::PingResponse => PacketType::new(t, 0),

            ControlType::Disconnect => PacketType::new(t, 0),
        }
    }

    pub fn to_u8(&self) -> u8 {
        (self.control_type as u8) << 4 | (self.flags & 0x0F)
    }

    pub fn from_u8(val: u8) -> Result<PacketType, PacketTypeError> {

        let type_val = val >> 4;
        let flag = val & 0x0F;

        macro_rules! vconst {
            ($flag:expr, $ret:path) => (
                if flag != $flag {
                    Err(PacketTypeError::InvalidFlag($ret, flag))
                } else {
                    Ok(PacketType::new($ret, flag))
                }
            )
        }

        match type_val {
            value::CONNECT => vconst!(0x00, ControlType::Connect),
            value::CONNACK => vconst!(0x00, ControlType::ConnectAcknowledgement),

            value::PUBLISH => Ok(PacketType::new(ControlType::Publish, flag)),
            value::PUBACK => vconst!(0x00, ControlType::PublishAcknowledgement),
            value::PUBREC => vconst!(0x00, ControlType::PublishReceived),
            value::PUBREL => vconst!(0x02, ControlType::PublishRelease),
            value::PUBCOMP => vconst!(0x00, ControlType::PublishComplete),

            value::SUBSCRIBE => vconst!(0x02, ControlType::Subscribe),
            value::SUBACK => vconst!(0x00, ControlType::SubscribeAcknowledgement),

            value::UNSUBSCRIBE => vconst!(0x02, ControlType::Unsubscribe),
            value::UNSUBACK => vconst!(0x00, ControlType::UnsubscribeAcknowledgement),

            value::PINGREQ => vconst!(0x00, ControlType::PingRequest),
            value::PINGRESP => vconst!(0x00, ControlType::PingResponse),

            value::DISCONNECT => vconst!(0x00, ControlType::Disconnect),

            0 | 15 => Err(PacketTypeError::ReservedType(type_val, flag)),
            _ => Err(PacketTypeError::UndefinedType(type_val, flag)),
        }
    }
}

#[derive(Debug)]
pub enum PacketTypeError {
    ReservedType(u8, u8),
    UndefinedType(u8, u8),
    InvalidFlag(ControlType, u8),
}

impl fmt::Display for PacketTypeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &PacketTypeError::ReservedType(t, flag) =>
                write!(f, "Reserved type {:?} ({:#X})", t, flag),
            &PacketTypeError::InvalidFlag(t, flag) =>
                write!(f, "Invalid flag for {:?} ({:#X})", t, flag),
            &PacketTypeError::UndefinedType(t, flag) =>
                write!(f, "Undefined type {:?} ({:#X})", t, flag),
        }
    }
}

impl Error for PacketTypeError {
    fn description(&self) -> &str {
        match self {
            &PacketTypeError::ReservedType(..) => "Reserved type",
            &PacketTypeError::UndefinedType(..) => "Undefined type",
            &PacketTypeError::InvalidFlag(..) => "Invalid flag",
        }
    }
}

mod value {
    pub const CONNECT: u8 = 1;
    pub const CONNACK: u8 = 2;
    pub const PUBLISH: u8 = 3;
    pub const PUBACK: u8 = 4;
    pub const PUBREC: u8 = 5;
    pub const PUBREL: u8 = 6;
    pub const PUBCOMP: u8 = 7;
    pub const SUBSCRIBE: u8 = 8;
    pub const SUBACK: u8 = 9;
    pub const UNSUBSCRIBE: u8 = 10;
    pub const UNSUBACK: u8 = 11;
    pub const PINGREQ: u8 = 12;
    pub const PINGRESP: u8 = 13;
    pub const DISCONNECT: u8 = 14;
}
