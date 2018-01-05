use std::io::{self, ErrorKind, Cursor};
use std::error::Error;
use bytes::BytesMut;
use tokio_io::codec::{Encoder, Decoder};

use mqtt3::{self, Packet, MqttWrite, MqttRead};

#[derive(Debug)]
pub struct MqttCodec;

impl Decoder for MqttCodec {
    type Item = Packet;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> io::Result<Option<Packet>> {
        // NOTE: `decode` might be called with `buf.len == 0` when prevous
        // decode call read all the bytes in the stream. We should return
        // Ok(None) in those cases or else the `read` call will return
        // Ok(0) => translated to UnexpectedEOF by `byteorder` crate.
        // `read` call Ok(0) happens when buffer specified was 0 bytes in len
        // https://doc.rust-lang.org/std/io/trait.Read.html#tymethod.read
        if buf.len() < 2 {
            return Ok(None);
        }

        let (packet, len) = {
            let mut buf_ref = buf.as_ref();
            match buf_ref.read_packet_with_len() {
                Err(e) => {
                    if let mqtt3::Error::Io(e) = e {
                        match e.kind() {
                            ErrorKind::TimedOut | ErrorKind::WouldBlock | ErrorKind::UnexpectedEof => return Ok(None),
                            _ => {
                                error!("mqtt3 io error = {:?}", e);
                                return Err(io::Error::new(e.kind(), e.description()))
                            },
                        }
                    } else {
                        error!("mqtt3 read error = {:?}", e);
                        return Err(io::Error::new(ErrorKind::Other, e.description()));
                    }
                }
                Ok(v) => v,
            }
        };

        // NOTE: It's possible that `decode` got called before `buf` has full bytes
        // necessary to frame raw bytes into a packet. In that case return Ok(None)
        // and the next time decode` gets called, there will be more bytes in `buf`,
        // hopefully enough to frame the packet
        if buf.len() < len {
            return Ok(None);
        }

        // println!("buf = {:?}", buf);
        // println!("{:?}, {:?}, {:?}", len, packet, buf.len());

        buf.split_to(len);

        Ok(Some(packet))
    }
}

impl Encoder for MqttCodec {
    type Item = Packet;
    type Error = io::Error;

    fn encode(&mut self, msg: Packet, buf: &mut BytesMut) -> io::Result<()> {
        let mut stream = Cursor::new(Vec::new());

        // TODO: Implement `write_packet` for `&mut BytesMut`
        if let Err(e) = stream.write_packet(&msg) {
            error!("Encode error. Error = {:?}", e);
            return Err(io::Error::new(io::ErrorKind::Other, "Unable to encode!"));
        }

        buf.extend(stream.get_ref());

        Ok(())
    }
}