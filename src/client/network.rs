use std::io::{self, Read, Write};

use tokio_core::net::TcpStream;
use tokio_tls::TlsStream;
use tokio_io::{AsyncRead, AsyncWrite};
use futures::Poll;

pub enum NetworkStream {
    Tcp(TcpStream),
    Tls(TlsStream<TcpStream>)
}

impl Read for NetworkStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.read(buf),
            NetworkStream::Tls(ref mut s) => s.read(buf),
        }
    }
}

impl Write for NetworkStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.write(buf),
            NetworkStream::Tls(ref mut s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.flush(),
            NetworkStream::Tls(ref mut s) => s.flush(),
        }
    }
}

impl AsyncRead for NetworkStream{}
impl AsyncWrite for NetworkStream{
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.shutdown(),
            NetworkStream::Tls(ref mut s) => s.shutdown(),
        }
    }
}