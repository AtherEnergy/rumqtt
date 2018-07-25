use std::net::{TcpStream, Shutdown, SocketAddr, ToSocketAddrs};
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::path::{Path, PathBuf};
use std::time::Duration;

use openssl::ssl::{self, SslMethod, SSL_VERIFY_NONE, SSL_VERIFY_PEER};
use openssl::x509::X509_FILETYPE_PEM;

use mqtt311::{MqttWrite, MqttRead};
pub type SslStream = ssl::SslStream<TcpStream>;

use error::{Result, Error};

pub struct SslContext {
    pub inner: Arc<ssl::SslConnector>,
}

impl SslContext {
    pub fn new<CA, C, K>(ca: CA, client_pair: Option<(C, K)>, should_verify_ca: bool) -> Result<Self>
    where
        CA: AsRef<Path>,
        C: AsRef<Path>,
        K: AsRef<Path>,
    {
        let mut ctx_builder = ssl::SslConnectorBuilder::new(SslMethod::tls())?;
        ctx_builder.set_ca_file(ca.as_ref())?;

        if let Some((cert, key)) = client_pair {
            ctx_builder.set_certificate_file(cert, X509_FILETYPE_PEM)?;
            ctx_builder.set_private_key_file(key, X509_FILETYPE_PEM)?;
        }

        if should_verify_ca {
            ctx_builder.set_verify(SSL_VERIFY_PEER);
        } else {
            ctx_builder.set_verify(SSL_VERIFY_NONE);
        }

        Ok(SslContext { inner: Arc::new(ctx_builder.build()) })
    }

    pub fn connect(&self, domain: &str, stream: TcpStream) -> Result<SslStream> {
        let ssl_stream = if domain == "" {
            warn!("Running without hostname verification. Your connection might not be secure");
            ssl::SslConnector::danger_connect_without_providing_domain_for_certificate_verification_and_server_name_indication(&*self.inner, stream)?
        } else {
            ssl::SslConnector::connect(&*self.inner, domain, stream)?
        };

        Ok(ssl_stream)
    }
}

pub enum NetworkStream {
    Tcp(TcpStream),
    Tls(SslStream),
    None,
}

impl NetworkStream {
    pub fn connect_timeout(addr: &str, ca: Option<PathBuf>, certs: Option<(PathBuf, PathBuf)>, host_name_verification: bool, timeout: Duration) -> Result<NetworkStream> {
        let domain = addr.split(":")
                         .map(str::to_string)
                         .next()
                         .unwrap_or_default();

        let addrs: Vec<SocketAddr> = addr.to_socket_addrs()?.collect();
        let addr = match addrs.get(0) {
            Some(a) => a,
            None => {
                error!("Dns resolve array empty");
                return Err(Error::DnsListEmpty)
            }
        };

        let stream = TcpStream::connect_timeout(&addr, timeout)?;
        warn!("Tcp connection successful");

        //NOTE: Should be less than default keep alive time to make sure that server doesn't 
        //      disconnect while waiting for read.
        stream.set_read_timeout(Some(Duration::new(10, 0)))?;
        stream.set_write_timeout(Some(Duration::new(30, 0)))?;
        
        let stream = if let Some(ca) = ca {
            let ssl_ctx = if let Some((ref crt, ref key)) = certs {
                SslContext::new(ca, Some((crt, key)), true)?
            } else {
                SslContext::new(ca, None::<(String, String)>, true)?
            };

            if host_name_verification {
                NetworkStream::Tls(ssl_ctx.connect(&domain, stream)?)
            } else {
                NetworkStream::Tls(ssl_ctx.connect("", stream)?)
            }
        } else {
            NetworkStream::Tcp(stream)
        };

        warn!("Tls connection successful");
        Ok(stream)
    }


    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        match *self {
            NetworkStream::Tcp(ref s) => s.shutdown(how),
            NetworkStream::Tls(ref s) => s.get_ref().shutdown(how),
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }

    pub fn set_read_timeout(&mut self, dur: Option<Duration>) -> io::Result<()> {
        match *self {
            NetworkStream::Tcp(ref s) => s.set_read_timeout(dur),
            NetworkStream::Tls(ref s) => s.get_ref().set_read_timeout(dur),
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }

    pub fn set_write_timeout(&mut self, dur: Option<Duration>) -> io::Result<()> {
        match *self {
            NetworkStream::Tcp(ref s) => s.set_write_timeout(dur),
            NetworkStream::Tls(ref s) => s.get_ref().set_write_timeout(dur),
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }
}

impl Read for NetworkStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.read(buf),
            NetworkStream::Tls(ref mut s) => s.read(buf),
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }
}

impl Write for NetworkStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.write(buf),
            NetworkStream::Tls(ref mut s) => s.write(buf),
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.flush(),
            NetworkStream::Tls(ref mut s) => s.flush(),
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }
}

impl MqttRead for NetworkStream {}
impl MqttWrite for NetworkStream {}
