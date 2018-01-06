use std::net::TcpStream;
use std::fs::File;
use std::io::{self, Read, Write, BufReader};
use std::sync::Arc;
use std::net::Shutdown;
use std::path::Path;
use std::time::Duration;

use rustls::{PrivateKey, Certificate, Stream, ClientConfig, ClientSession, TLSError};
use rustls::internal::pemfile;
use error::{Result, Error};

quick_error! {
    #[derive(Debug)]
    pub enum StreamError {
        RustlsError(err: TLSError) {
            from()
        }
    }
}

impl From<TLSError> for Error {
  fn from(e: TLSError) -> Error {
    Error::StreamError(e.into())
  }
}

pub struct SslStream {
  pub session: ClientSession,
  pub stream: TcpStream
}

impl SslStream {
  fn as_io<'a>(&'a mut self) -> Stream<'a, ClientSession, TcpStream> {
    Stream::new(&mut self.session, &mut self.stream)
  }
}

pub struct SslContext {
    pub inner: Arc<ClientConfig>,
}

impl SslContext {
    pub fn new<CA, C, K>(ca: CA, client_pair: Option<(C, K)>, _should_verify_ca: bool) -> Result<Self>
        where CA: AsRef<Path>,
              C: AsRef<Path>,
              K: AsRef<Path>
    {
        let mut ctx_builder = ClientConfig::new();

        let mut ca_file = try!(File::open(ca).map(|f| BufReader::new(f)));
        try!(
            ctx_builder.root_store
                .add_pem_file(&mut ca_file)
                .map_err(|_| TLSError::General("Unable to add CA file to root store".to_string()))
        );
        if let Some((cert, key)) = client_pair {
            let cert_chain = try!(certs_from_pem_file(cert));
            let key_der = try!(key_from_pem_file(key));
            ctx_builder.set_single_client_cert(cert_chain, key_der);
        }

        Ok(SslContext { inner: Arc::new(ctx_builder) })
    }

    pub fn connect(&self, domain: &str, stream: TcpStream) -> Result<SslStream> {
        let session = ClientSession::new(&self.inner, domain);
        let ssl_stream = SslStream { session, stream };
        Ok(ssl_stream)
    }
}

fn certs_from_pem_file<C: AsRef<Path>>(path: C) -> Result<Vec<Certificate>> {
    let mut cert_file = try!(File::open(path).map(|f| BufReader::new(f)));
    pemfile::certs(&mut cert_file)
      .map_err(|_| TLSError::General("Unable to read certs from pem file".to_string()).into())
}

fn key_from_pem_file<K: AsRef<Path>>(path: K) -> Result<PrivateKey> {
    let mut key_file = try!(File::open(path).map(|f| BufReader::new(f)));
    pemfile::rsa_private_keys(&mut key_file).and_then(|keys| {
        keys.into_iter().next().ok_or(())
    }).map_err(|_| {
      TLSError::General("Unable to read private key from pem file".to_string()).into()
    })
}

pub enum NetworkStream {
    Tcp(TcpStream),
    Tls(SslStream),
    None,
}

impl NetworkStream {
    // fn get_ref(&self) -> io::Result<&TcpStream> {
    //     match *self {
    //         NetworkStream::Tcp(ref s) => Ok(s),
    //         NetworkStream::Tls(ref s) => Ok(s.get_ref()),
    // NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No
    // stream!")),
    //     }
    // }

    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        match *self {
            NetworkStream::Tcp(ref s) => s.shutdown(how),
            NetworkStream::Tls(ref s) => s.stream.shutdown(how),
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }

    pub fn set_read_timeout(&mut self, dur: Option<Duration>) -> io::Result<()> {
        match *self {
            NetworkStream::Tcp(ref s) => s.set_read_timeout(dur),
            NetworkStream::Tls(ref s) => s.stream.set_read_timeout(dur),
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }

    pub fn set_write_timeout(&mut self, dur: Option<Duration>) -> io::Result<()> {
        match *self {
            NetworkStream::Tcp(ref s) => s.set_write_timeout(dur),
            NetworkStream::Tls(ref s) => s.stream.set_write_timeout(dur),
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }
}

impl Read for NetworkStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.read(buf),
            NetworkStream::Tls(ref mut s) => s.as_io().read(buf),
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }
}

impl Write for NetworkStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.write(buf),
            NetworkStream::Tls(ref mut s) => s.as_io().write(buf),
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.flush(),
            NetworkStream::Tls(ref mut s) => s.as_io().flush(),
            NetworkStream::None => Err(io::Error::new(io::ErrorKind::Other, "No stream!")),
        }
    }
}
