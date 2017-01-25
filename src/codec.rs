use std::io::{self, Cursor};
use tokio_core::io::{Codec, EasyBuf};

use mqtt3::{Packet, MqttWrite, MqttRead};

pub struct MqttCodec;

impl Codec for MqttCodec {
    type In = Packet;
    type Out = Packet;

    fn decode(&mut self, buf: &mut EasyBuf) -> Result<Option<Self::In>, io::Error> {
        let (packet, len) = {
            let mut buf_ref = buf.as_ref();
            match buf_ref.read_packet_with_len() {
                Err(..) => return Ok(None),
                Ok(v) => v,
            }
        };

        buf.drain_to(len);
        println!("{:?}, {:?}", len, packet);
        Ok(Some(packet))
    }

    fn encode(&mut self, msg: Self::Out, buf: &mut Vec<u8>) -> io::Result<()> {
        let mut stream = Cursor::new(Vec::new());

        if let Err(e) = stream.write_packet(&msg) {
            println!("{:?}", e);
            return Err(io::Error::new(io::ErrorKind::Other, "oh no!"));
        }

        for i in stream.get_ref() {
            buf.push(*i);
        }
        Ok(())
    }
}
