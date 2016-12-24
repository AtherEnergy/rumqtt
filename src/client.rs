use std::io::{self, BufReader, Cursor};
use core::io::{Codec, EasyBuf, Framed, Io};
use proto::pipeline::ClientProto;

use mqtt3::{Packet, MqttWrite, MqttRead, Error as MqttError};

// impl MqttRead for EasyBuf{}

pub struct MqttCodec;

impl Codec for MqttCodec {
    type In = Packet;
    type Out = Packet;

    fn decode(&mut self, buf: &mut EasyBuf) -> Result<Option<Self::In>, io::Error> {
        let buf = buf.as_ref();
        let mut buf = BufReader::new(buf);
        let packet = buf.read_packet().unwrap();
        println!("{:?}", packet);
        Ok(Some(packet))
    }

    fn encode(&mut self, msg: Self::Out, buf: &mut Vec<u8>) -> io::Result<()> {
        let mut stream = Cursor::new(Vec::new());
        stream.write_packet(&msg);
        for i in stream.get_ref() {
            buf.push(*i);
        }
        Ok(())
    }
}

pub struct MqttClientProto;

impl<T: Io + 'static> ClientProto<T> for MqttClientProto {
    type Request = Packet;
    type Response = Packet;
    type Error = io::Error;
    type Transport = Framed<T, MqttCodec>;
    type BindTransport = Result<Self::Transport, io::Error>;

    fn bind_transport(&self, io: T) -> Self::BindTransport {
        Ok(io.framed(MqttCodec))
    }
}
