use mio::tcp::TcpStream;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::path::Path;
use std::net::Shutdown;
use error::{Error, Result};
use openssl::ssl::{self, SslMethod, SSL_VERIFY_NONE};
use openssl::x509::X509FileType;


pub type SslStream = ssl::SslStream<TcpStream>;
pub type SslError = ssl::error::SslError;

#[derive(Debug, Clone)]
pub struct SslContext {
    pub inner: Arc<ssl::SslContext>,
}

impl Default for SslContext {
    fn default() -> SslContext { SslContext { inner: Arc::new(ssl::SslContext::new(SslMethod::Tlsv1_2).unwrap()) } }
}

impl SslContext {
    // fn new(context: ssl::SslContext) -> Self { SslContext { inner:
    // Arc::new(context) } }

    // fn with_cert_and_key<C, K>(cert: C, key: K) -> Result<SslContext>
    //     where C: AsRef<Path>,
    //           K: AsRef<Path>
    // {
    //     let mut ctx = try!(ssl::SslContext::new(SslMethod::Tlsv1_2));
    //     try!(ctx.set_cipher_list("DEFAULT"));
    //     try!(ctx.set_certificate_file(cert.as_ref(), X509FileType::PEM));
    //     try!(ctx.set_private_key_file(key.as_ref(), X509FileType::PEM));
    //     ctx.set_verify(SSL_VERIFY_NONE, None);
    //     Ok(SslContext { inner: Arc::new(ctx) })
    // }

    /// Create a new `SslContext` with server authentication
    pub fn with_ca<CA>(ca: CA) -> Result<SslContext>
        where CA: AsRef<Path>
    {
        let mut ctx = try!(ssl::SslContext::new(SslMethod::Tlsv1_2));
        try!(ctx.set_cipher_list("DEFAULT"));
        try!(ctx.set_CA_file(ca.as_ref()));
        ctx.set_verify(SSL_VERIFY_NONE, None);
        Ok(SslContext { inner: Arc::new(ctx) })
    }

    /// Create a new `SslContext` with client and server authentication
    pub fn with_cert_key_and_ca<C, K, CA>(cert: C, key: K, ca: CA) -> Result<SslContext>
        where C: AsRef<Path>,
              K: AsRef<Path>,
              CA: AsRef<Path>
    {
        let mut ctx = try!(ssl::SslContext::new(SslMethod::Tlsv1_2));
        try!(ctx.set_cipher_list("DEFAULT"));
        try!(ctx.set_certificate_file(cert.as_ref(), X509FileType::PEM));
        try!(ctx.set_private_key_file(key.as_ref(), X509FileType::PEM));
        try!(ctx.set_CA_file(ca.as_ref()));
        ctx.set_verify(SSL_VERIFY_NONE, None);
        Ok(SslContext { inner: Arc::new(ctx) })
    }

    /// Create a new TLS1.2 connection
    pub fn connect(&self, stream: TcpStream) -> Result<SslStream> {
        match ssl::SslStream::connect(&*self.inner, stream) {
            Ok(stream) => Ok(stream),
            Err(err) => Err(io::Error::new(io::ErrorKind::ConnectionAborted, err).into()),
        }
    }
}

//TODO: Make this a trait
pub enum NetworkStream {
    Tcp(TcpStream),
    Ssl(SslStream),
    None,
}

impl NetworkStream {
    // pub fn peer_addr(&self) -> Result<SocketAddr> {
    //     match *self {
    //         NetworkStream::Tcp(ref s) => {
    //             let addr = try!(s.peer_addr());
    //             Ok(addr)
    //         }
    //         NetworkStream::Ssl(ref s) => {
    //             let addr = try!(s.get_ref().peer_addr());
    //             Ok(addr)
    //         }
    //         NetworkStream::None => Err(Error::NoStreamError),
    //     }
    // }

    pub fn shutdown(&self, how: Shutdown) -> Result<()> {
        match *self {
            NetworkStream::Tcp(ref s) => {
                try!(s.shutdown(how));
                Ok(())
            }
            NetworkStream::Ssl(ref s) => {
                try!(s.get_ref().shutdown(how));
                Ok(())
            }
            NetworkStream::None => Err(Error::NoStream),
        }
    }

    pub fn get_ref(&self) -> Result<&TcpStream> {
        match *self {
            NetworkStream::Tcp(ref s) => Ok(s),
            NetworkStream::Ssl(ref s) => Ok(s.get_ref()),
            NetworkStream::None => Err(Error::NoStream),
        }
    }
}

impl Read for NetworkStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.read(buf),
            NetworkStream::Ssl(ref mut s) => s.read(buf),
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }
}

impl Write for NetworkStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.write(buf),
            NetworkStream::Ssl(ref mut s) => s.write(buf),
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.flush(),
            NetworkStream::Ssl(ref mut s) => s.flush(),
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }
}
