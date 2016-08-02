use mio::tcp::TcpStream;
use std::io::{self, Read, Write, BufReader};
use std::fs::File;
use std::sync::Arc;
use std::net::Shutdown;
use std::path::Path;

use rustls::{self, Session};
use error::Result;

pub struct TlsStream {
    stream: TcpStream,
    pub tls_session: rustls::ClientSession,
}

impl TlsStream {
    pub fn make_config<P>(cafile: P) -> Result<rustls::ClientConfig>
        where P: AsRef<Path>
    {
        let mut config = rustls::ClientConfig::new();
        let certfile = try!(File::open(cafile));
        let mut reader = BufReader::new(certfile);
        config.root_store.add_pem_file(&mut reader).unwrap();
        Ok(config)
    }

    pub fn new(stream: TcpStream, hostname: &str, cfg: rustls::ClientConfig) -> Self {
        let cfg = Arc::new(cfg);
        TlsStream {
            stream: stream,
            // NOTE: Hostname should match to server address or else --> Decode Error
            tls_session: rustls::ClientSession::new(&cfg, hostname),
        }
    }
}

pub enum NetworkStream {
    Tcp(TcpStream),
    Tls(TlsStream),
    None,
}

impl NetworkStream {
    pub fn get_ref(&self) -> io::Result<&TcpStream> {
        match *self {
            NetworkStream::Tcp(ref s) => Ok(s),
            NetworkStream::Tls(ref s) => Ok(&s.stream),
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }

    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        match *self {
            NetworkStream::Tcp(ref s) => s.shutdown(how),
            NetworkStream::Tls(ref s) => s.stream.shutdown(how),
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }
}

impl Read for NetworkStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.read(buf),
            NetworkStream::Tls(ref mut s) => {
                while s.tls_session.wants_read() {
                    match s.tls_session.read_tls(&mut s.stream) {
                        Ok(_) => {
                            match s.tls_session.process_new_packets() {
                                Ok(_) => (),
                                Err(e) => return Err(io::Error::new(io::ErrorKind::Other, format!("{:?}", e))),
                            }
                            while s.tls_session.wants_write() {
                                try!(s.tls_session.write_tls(&mut s.stream));
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
                s.tls_session.read(buf)
            }
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }
}

impl Write for NetworkStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.write(buf),
            NetworkStream::Tls(ref mut s) => {
                let res = s.tls_session.write(buf);
                while s.tls_session.wants_write() {
                    try!(s.tls_session.write_tls(&mut s.stream));
                }
                res
            }
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.flush(),
            NetworkStream::Tls(ref mut s) => s.tls_session.flush(),
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }
}
