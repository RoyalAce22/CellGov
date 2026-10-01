//! The two wire protocols the console speaks, over one byte stream.
//!
//! webMAN answers HTTP/1.0 on port 80 and anonymous FTP on port 21.
//! Both clients read and write through [`Wire`], which a
//! [`std::net::TcpStream`] implements in the binary and a pair of
//! [`std::io::Cursor`]s implements in a test, so no unit test opens a
//! connection.

use std::io::{Read, Write};

pub mod ftp;
pub mod http;

/// A byte stream to the console.
pub trait Wire: Read + Write {}

impl<T: Read + Write> Wire for T {}

/// Where the console listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// Host name or address.
    pub host: String,
    /// The HTTP port webMAN serves on.
    pub http_port: u16,
    /// The FTP port webMAN serves on.
    pub ftp_port: u16,
    /// Read and write timeout for one exchange, in milliseconds.
    pub io_timeout_ms: u64,
}

impl Endpoint {
    /// The console at `host` on webMAN's default ports.
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            http_port: 80,
            ftp_port: 21,
            io_timeout_ms: 10_000,
        }
    }
}
