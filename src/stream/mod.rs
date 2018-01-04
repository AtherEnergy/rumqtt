/* cfg(openssl)
mod openssl;
pub use self::openssl::{SslContext, NetworkStream};
*/

/* cfg(rustls) */
mod rustls;
pub use self::rustls::{SslContext, NetworkStream};
/* */
