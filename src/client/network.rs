use std::io;

use crate::client::network::stream::NetworkStream;
use serde_derive::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite};

pub mod stream {
    use crate::{
        client::network::{generate_httpproxy_auth, resolve},
        codec::MqttCodec,
        error::ConnectError,
    };
    use futures::sink::SinkExt;
    use std::{
        io::{self, BufReader, Cursor},
        sync::Arc,
    };
    use tokio::net::TcpStream;
    use tokio_rustls::{
        client::TlsStream,
        rustls::{internal::pemfile, ClientConfig},
        TlsConnector,
    };
    use tokio_util::codec::{Decoder, Framed, LinesCodec};
    use webpki::DNSNameRef;

    #[allow(clippy::large_enum_variant)]
    pub enum NetworkStream {
        Tcp(TcpStream),
        Tls(TlsStream<TcpStream>),
    }

    impl NetworkStream {
        pub fn builder() -> NetworkStreamBuilder {
            NetworkStreamBuilder {
                certificate_authority: None,
                client_cert: None,
                client_private_key: None,
                alpn_protocols: Vec::new(),
                http_proxy: None,
            }
        }
    }

    #[derive(Clone)]
    struct HttpProxy {
        id: String,
        proxy_host: String,
        proxy_port: u16,
        key: Vec<u8>,
        expiry: i64,
    }

    pub struct NetworkStreamBuilder {
        certificate_authority: Option<Vec<u8>>,
        client_cert: Option<Vec<u8>>,
        client_private_key: Option<Vec<u8>>,
        alpn_protocols: Vec<Vec<u8>>,
        http_proxy: Option<HttpProxy>,
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

        pub fn add_alpn_protocols(mut self, protocols: &[Vec<u8>]) -> NetworkStreamBuilder {
            self.alpn_protocols.append(&mut protocols.to_vec());
            debug!("{:?}", &self.alpn_protocols);
            self
        }

        pub fn set_http_proxy(
            mut self,
            id: &str,
            proxy_host: &str,
            proxy_port: u16,
            key: &[u8],
            expiry: i64,
        ) -> NetworkStreamBuilder {
            self.http_proxy = Some(HttpProxy {
                id: id.to_owned(),
                proxy_host: proxy_host.to_owned(),
                proxy_port: proxy_port,
                key: key.to_owned(),
                expiry,
            });

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

            config.set_protocols(&self.alpn_protocols);

            Ok(TlsConnector::from(Arc::new(config)))
        }

        #[allow(clippy::too_many_arguments)]
        pub async fn http_connect(
            &self,
            id: &str,
            proxy_host: &str,
            proxy_port: u16,
            host: &str,
            port: u16,
            key: &[u8],
            expiry: i64,
        ) -> Result<TcpStream, io::Error> {
            use tokio_util::codec::LinesCodecError;
            let proxy_auth = generate_httpproxy_auth(id, key, expiry);
            let connect = format!(
                "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\nProxy-Authorization: {}\r\n\r\n",
                host, port, host, port, proxy_auth
            );
            debug!("{}", connect);

            let codec = LinesCodec::new();
            let proxy_address = resolve(proxy_host, proxy_port)?;

            let mut tcp = TcpStream::connect(&proxy_address).await?;
            let mut f = Decoder::framed(codec, &mut tcp);
            f.send(connect).await.map_err(|e| match e {
                LinesCodecError::MaxLineLengthExceeded => unreachable!(),
                LinesCodecError::Io(e) => e,
            })?;
            Ok(tcp)
        }

        pub async fn tcp_connect(&self, host: &str, port: u16) -> Result<TcpStream, io::Error> {
            let addr = resolve(host, port)?;
            TcpStream::connect(&addr).await
        }

        pub async fn connect(mut self, host: &str, port: u16) -> Result<Framed<NetworkStream, MqttCodec>, ConnectError> {
            let tls_connector = self.create_stream();
            let host_tcp = host.to_owned();
            let http_proxy = self.http_proxy.clone();
            let stream = match http_proxy {
                Some(HttpProxy {
                    id,
                    proxy_host,
                    proxy_port,
                    key,
                    expiry,
                }) => {
                    self.http_connect(&id, &proxy_host, proxy_port, &host_tcp, port, &key, expiry)
                        .await?
                }
                None => self.tcp_connect(host, port).await?,
            };

            Ok(match tls_connector {
                Ok(tls_connector) => {
                    let domain = DNSNameRef::try_from_ascii_str(&host).unwrap().to_owned();
                    let stream = tls_connector.connect(domain.as_ref(), stream).await?;
                    let stream = NetworkStream::Tls(stream);
                    MqttCodec.framed(stream)
                }
                Err(ConnectError::NoCertificateAuthority) => {
                    let stream = NetworkStream::Tcp(stream);
                    MqttCodec.framed(stream)
                }
                _ => unimplemented!(),
            })
        }
    }
}

fn resolve(host: &str, port: u16) -> Result<SocketAddr, io::Error> {
    use std::net::ToSocketAddrs;

    (host, port).to_socket_addrs().and_then(|mut addrs| {
        addrs.next().ok_or_else(|| {
            let err_msg = format!("invalid hostname '{}'", host);
            io::Error::new(io::ErrorKind::Other, err_msg)
        })
    })
}

fn generate_httpproxy_auth(id: &str, key: &[u8], expiry: i64) -> String {
    use chrono::{Duration, Utc};
    use jsonwebtoken::{encode, Algorithm, Header, EncodingKey};
    use uuid::Uuid;

    #[derive(Debug, Serialize, Deserialize)]
    struct Claims {
        iat: i64,
        exp: i64,
        jti: Uuid,
    }

    let time = Utc::now();
    let jwt_header = Header::new(Algorithm::RS256);
    let iat = time.timestamp();
    let jti = Uuid::new_v4();

    let exp = time
        .checked_add_signed(Duration::minutes(expiry))
        .expect("Unable to create expiry")
        .timestamp();

    let claims = Claims { iat, exp, jti };

    let jwt = encode(&jwt_header, &claims, &EncodingKey::from_secret(&key)).unwrap();
    let userid_password = format!("{}:{}", id, jwt);
    let auth = base64::encode(userid_password.as_bytes());

    format!("Basic {}", auth)
}

impl AsyncRead for NetworkStream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        match *self {
            NetworkStream::Tcp(ref mut s) => Pin::new(s).poll_read(cx, buf),
            NetworkStream::Tls(ref mut s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for NetworkStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<Result<usize, io::Error>> {
        match *self {
            NetworkStream::Tcp(ref mut s) => Pin::new(s).poll_write(cx, buf),
            NetworkStream::Tls(ref mut s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), io::Error>> {
        match *self {
            NetworkStream::Tcp(ref mut s) => Pin::new(s).poll_flush(cx),
            NetworkStream::Tls(ref mut s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), io::Error>> {
        match *self {
            NetworkStream::Tcp(ref mut s) => Pin::new(s).poll_shutdown(cx),
            NetworkStream::Tls(ref mut s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

mod test {
    #[test]
    fn resolve() {
        use super::resolve;
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

        let localhost_v4 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 1883);
        let localhost_v6 = SocketAddr::new(IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1)), 1883);

        assert_eq!(resolve("127.0.0.1", 1883).unwrap(), localhost_v4);
        assert_eq!(resolve("::1", 1883).unwrap(), localhost_v6);

        // localhost resolvs to a v4 or v6 address depending on host settings
        let addr = resolve("localhost", 1883).unwrap();
        assert!(addr == localhost_v4 || addr == localhost_v6);
    }
}
