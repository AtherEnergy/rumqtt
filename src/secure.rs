use mio::tcp::TcpStream;
use std::io::{self, Read, Write, BufReader};
use std::fs::File;
use std::sync::Arc;
use std::path::Path;
use std::net::{SocketAddr, ToSocketAddrs, Shutdown};

use rustls;

pub struct TlsStream {
    stream: TcpStream,
    closing: bool,
    clean_closure: bool,
    tls_session: rustls::ClientSession,
}

impl TlsStream {
    fn make_config() -> rustls::ClientConfig {
        let mut config = rustls::ClientConfig::new();
        let certfile = File::open("/Users/ravitejareddy/Downloads/ca.crt").unwrap();
        let mut reader = BufReader::new(certfile);
        config.root_store
            .add_pem_file(&mut reader)
            .unwrap();

        config
    }

    pub fn new(stream: TcpStream, hostname: &str, cfg: rustls::ClientConfig) -> Self {
        let cfg = Arc::new(cfg);
        TlsStream {
            stream: stream,
            closing: false,
            clean_closure: false,
            tls_session: rustls::ClientSession::new(&cfg, hostname),
        }
    }
}

pub enum NetworkStream {
    Tcp(TcpStream),
    Tls(TlsStream),
}

impl NetworkStream {
    pub fn get_ref(&self) -> io::Result<&TcpStream> {
        match *self {
            NetworkStream::Tcp(ref s) => Ok(s),
            NetworkStream::Tls(ref s) => Ok(&s.stream),
        }
    }

    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        match *self {
            NetworkStream::Tcp(ref s) => s.shutdown(how),
            NetworkStream::Tls(ref s) => s.stream.shutdown(how),
        }
    }
}

impl Read for NetworkStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.read(buf),
            NetworkStream::Tls(ref mut s) => s.tls_session.read(buf),
        }
    }
}

impl Write for NetworkStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.write(buf),
            NetworkStream::Tls(ref mut s) => s.tls_session.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.flush(),
            NetworkStream::Tls(ref mut s) => s.tls_session.flush(),
        }
    }
}