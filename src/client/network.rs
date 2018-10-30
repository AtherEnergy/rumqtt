use std::io::{self, Read, Write};

use client::network::stream::NetworkStream;
use futures::Poll;
use std::net::SocketAddr;
use tokio_io::{AsyncRead, AsyncWrite};

#[cfg(feature = "rustls")]
pub mod stream {
    use client::network::lookup_ipv4;
    use codec::MqttCodec;
    use error::ConnectError;
    use futures::future;
    use futures::{future::Either, Future};
    use std::io::BufReader;
    use std::io::Cursor;
    use std::sync::Arc;
    use tokio::net::TcpStream;
    use tokio_codec::{Decoder, Framed};
    use tokio_rustls::{rustls::internal::pemfile, rustls::ClientConfig, TlsConnector};
    use tokio_rustls::{rustls::ClientSession, TlsStream};
    use webpki::DNSNameRef;

    pub enum NetworkStream {
        Tcp(TcpStream),
        Tls(TlsStream<TcpStream, ClientSession>),
    }

    impl NetworkStream {
        pub fn builder() -> NetworkStreamBuilder {
            NetworkStreamBuilder { certificate_authority: None,
                                   client_cert: None,
                                   client_private_key: None }
        }
    }

    pub struct NetworkStreamBuilder {
        certificate_authority: Option<Vec<u8>>,
        client_cert: Option<Vec<u8>>,
        client_private_key: Option<Vec<u8>>,
    }

    impl NetworkStreamBuilder {
        pub fn add_certificate_authority(mut self, ca: &[u8]) -> NetworkStreamBuilder {
            self.certificate_authority = Some(ca.to_vec());
            self
        }

        pub fn add_client_auth(mut self, cert: &[u8], private_key: &[u8]) -> NetworkStreamBuilder {
            self.client_cert = Some(cert.to_vec());
            self.client_private_key = Some(private_key.to_vec());
            self
        }

        fn create_stream(&mut self) -> Result<TlsConnector, ConnectError> {
            let mut config = ClientConfig::new();

            match self.certificate_authority.clone() {
                Some(ca) => {
                    let mut ca = BufReader::new(Cursor::new(ca));
                    config.root_store.add_pem_file(&mut ca).unwrap();
                }
                None => return Err(ConnectError::NoCertificateAuthority),
            }

            match (self.client_cert.clone(), self.client_private_key.clone()) {
                (Some(cert), Some(key)) => {
                    let mut cert = BufReader::new(Cursor::new(cert));
                    let mut keys = BufReader::new(Cursor::new(key));

                    let certs = pemfile::certs(&mut cert).unwrap();
                    let keys = pemfile::rsa_private_keys(&mut keys).unwrap();

                    config.set_single_client_cert(certs, keys[0].clone());
                }
                (None, None) => (),
                _ => unimplemented!(),
            };

            Ok(TlsConnector::from(Arc::new(config)))
        }

        pub fn connect(
            mut self,
            host: &str,
            port: u16)
            -> impl Future<Item = Framed<NetworkStream, MqttCodec>, Error = ConnectError> {
            // let host = host.to_owned();
            let addr = lookup_ipv4(host, port);

            let tls_connector = self.create_stream();

            let network_future = match tls_connector {
                Ok(tls_connector) => {
                    let domain = DNSNameRef::try_from_ascii_str(host).unwrap().to_owned();
                    Either::A(TcpStream::connect(&addr).and_then(move |stream| {
                                                           tls_connector.connect(domain.as_ref(),
                                                                                 stream)
                                                       })
                                                       .map_err(ConnectError::from)
                                                       .and_then(|stream| {
                                                           let stream = NetworkStream::Tls(stream);
                                                           future::ok(MqttCodec.framed(stream))
                                                       }))
                }
                Err(ConnectError::NoCertificateAuthority) => {
                    Either::B(TcpStream::connect(&addr).and_then(|stream| {
                                                           let stream = NetworkStream::Tcp(stream);
                                                           future::ok(MqttCodec.framed(stream))
                                                       })
                                                       .map_err(ConnectError::from))
                }
                _ => unimplemented!(),
            };

            network_future
        }
    }
}

#[cfg(feature = "nativetls")]
mod stream {
    use tokio::net::TcpStream;
    use tokio_tls::TlsStream;

    pub enum NetworkStream {
        Tcp(TcpStream),
        Tls(TlsStream<TcpStream>),
    }

    impl NetworkStream {}
}

fn lookup_ipv4(host: &str, port: u16) -> SocketAddr {
    use std::net::ToSocketAddrs;

    let addrs = (host, port).to_socket_addrs().unwrap();
    for addr in addrs {
        if let SocketAddr::V4(_) = addr {
            return addr;
        }
    }

    unreachable!("Cannot lookup address");
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

impl AsyncRead for NetworkStream {}
impl AsyncWrite for NetworkStream {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.shutdown(),
            NetworkStream::Tls(ref mut s) => s.shutdown(),
        }
    }
}
