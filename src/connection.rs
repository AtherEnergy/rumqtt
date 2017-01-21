use std::net::{SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};
use std::thread;

use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;
use tokio_core::io::{Io, Framed};
use futures::Future;
use futures::Sink;
use futures::Stream;

use error::*;
use packet::*;
use codec::MqttCodec;
use clientoptions::MqttOptions;

pub struct Connection {
    pub addr: SocketAddr,
    pub state: MqttState,
    pub opts: MqttOptions,
    pub initial_connect: bool,
    pub await_pingresp: bool,
    pub last_flush: Instant,

    // This is a combination of Tcp Connection
    // and Encoder/Decoder functions
    pub framed: Option<Framed<TcpStream, MqttCodec>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttState {
    Handshake,
    Connected,
    Disconnected,
}

impl Connection {
    pub fn start(addr: SocketAddr, opts: MqttOptions) -> Result<Self> {
        let mut connection = Connection {
            addr: addr,
            state: MqttState::Disconnected,
            opts: opts,
            initial_connect: true,
            await_pingresp: false,
            last_flush: Instant::now(),
            framed: None,
        };

        connection._try_reconnect()?;
        Ok(connection)
    }

    fn _try_reconnect(&mut self) -> Result<()> {
        match self.opts.reconnect {
            // TODO: Implement
            None => unimplemented!(),
            Some(dur) => {
                if !self.initial_connect {
                    thread::sleep(Duration::new(dur as u64, 0));
                }

                let mut reactor = Core::new().unwrap();
                let handle = reactor.handle();

                let connect = generate_connect_packet("".to_string(), true, None, None);

                let f_response = TcpStream::connect(&self.addr, &handle).and_then(|connection| {
                    let framed = connection.framed(MqttCodec);
                    let f1 = framed.send(connect);

                    f1.and_then(|framed| {
                            framed.into_future()
                                .and_then(|(res, stream)| Ok((res, stream)))
                                .map_err(|(err, _stream)| err)
                        })
                        .boxed()
                });

                let response = reactor.run(f_response);
                let (packet, frame) = response?;
                println!("{:?}", packet);
                self.framed = Some(frame);
                Ok(())
            }
        }
    }
}
