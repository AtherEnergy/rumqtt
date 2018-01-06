#[cfg(feature = "tls-openssl")]
mod openssl;

#[cfg(feature = "tls-openssl")]
pub use self::openssl::{SslContext, NetworkStream, StreamError};

#[cfg(feature = "tls-rustls")]
mod rustls;

#[cfg(feature = "tls-rustls")]
pub use self::rustls::{SslContext, NetworkStream, StreamError};

#[cfg(not(any(feature = "tls-openssl", feature = "tls-rustls")))]
mod no_tls;

#[cfg(not(any(feature = "tls-openssl", feature = "tls-rustls")))]
pub use self::no_tls::{NetworkStream, StreamError};
