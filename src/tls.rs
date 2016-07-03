use mio::tcp::TcpStream;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::path::Path;
use std::net::{SocketAddr, ToSocketAddrs, Shutdown};

use openssl::ssl::{self, SslMethod, SSL_VERIFY_NONE};
use openssl::x509::X509FileType;


pub type SslStream = ssl::SslStream<TcpStream>;
pub type SslError = ssl::error::SslError;

#[derive(Debug, Clone)]
pub struct SslContext {
    pub inner: Arc<ssl::SslContext>,
}

impl Default for SslContext {
    fn default() -> SslContext {
        SslContext { inner: Arc::new(ssl::SslContext::new(SslMethod::Tlsv1_2).unwrap()) }
    }
}

impl SslContext {
    pub fn new(context: ssl::SslContext) -> Self {
        SslContext { inner: Arc::new(context) }
    }

    pub fn with_cert_and_key<C, K>(cert: C, key: K) -> Result<SslContext, SslError>
        where C: AsRef<Path>,
              K: AsRef<Path>
    {
        let mut ctx = try!(ssl::SslContext::new(SslMethod::Sslv23));
        try!(ctx.set_cipher_list("DEFAULT"));
        try!(ctx.set_certificate_file(cert.as_ref(), X509FileType::PEM));
        try!(ctx.set_private_key_file(key.as_ref(), X509FileType::PEM));
        ctx.set_verify(SSL_VERIFY_NONE, None);
        Ok(SslContext { inner: Arc::new(ctx) })
    }

    pub fn with_ca<CA>(ca: CA) -> Result<SslContext, SslError>
        where CA: AsRef<Path>
    {
        let mut ctx = try!(ssl::SslContext::new(SslMethod::Sslv23));
        try!(ctx.set_cipher_list("DEFAULT"));
        try!(ctx.set_CA_file(ca.as_ref()));
        ctx.set_verify(SSL_VERIFY_NONE, None);
        Ok(SslContext { inner: Arc::new(ctx) })
    }

    pub fn with_cert_key_and_ca<C, K, CA>(cert: C, key: K, ca: CA) -> Result<SslContext, SslError>
        where C: AsRef<Path>,
              K: AsRef<Path>,
              CA: AsRef<Path>
    {
        let mut ctx = try!(ssl::SslContext::new(SslMethod::Sslv23));
        try!(ctx.set_cipher_list("DEFAULT"));
        try!(ctx.set_certificate_file(cert.as_ref(), X509FileType::PEM));
        try!(ctx.set_private_key_file(key.as_ref(), X509FileType::PEM));
        try!(ctx.set_CA_file(ca.as_ref()));
        ctx.set_verify(SSL_VERIFY_NONE, None);
        Ok(SslContext { inner: Arc::new(ctx) })
    }

    pub fn connect(&self, stream: TcpStream) -> Result<SslStream, io::Error> {
        match ssl::SslStream::connect(&*self.inner, stream) {
            Ok(stream) => Ok(stream),
            Err(err) => Err(io::Error::new(io::ErrorKind::ConnectionAborted, err).into()),
        }
    }
}

pub enum NetworkStream {
    Tcp(TcpStream),
    Ssl(SslStream),
}

impl NetworkStream {
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        match *self {
            NetworkStream::Tcp(ref s) => s.peer_addr(),
            NetworkStream::Ssl(ref s) => s.get_ref().peer_addr(),
        }
    }

    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        match *self {
            NetworkStream::Tcp(ref s) => s.shutdown(how),
            NetworkStream::Ssl(ref s) => s.get_ref().shutdown(how),
        }
    }

    pub fn get_ref(&self) -> io::Result<&TcpStream> {
        match *self {
            NetworkStream::Tcp(ref s) => Ok(s),
            NetworkStream::Ssl(ref s) => Ok(s.get_ref()),
        }
    }
}

impl Read for NetworkStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.read(buf),
            NetworkStream::Ssl(ref mut s) => s.read(buf),
        }
    }
}

impl Write for NetworkStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.write(buf),
            NetworkStream::Ssl(ref mut s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.flush(),
            NetworkStream::Ssl(ref mut s) => s.flush(),
        }
    }
}
