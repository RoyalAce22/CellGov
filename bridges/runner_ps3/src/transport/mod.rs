//! The two wire protocols the console speaks, over one byte stream.
//!
//! webMAN answers HTTP/1.0 on port 80 and anonymous FTP on port 21.
//! Both clients read and write through [`Wire`], which a
//! [`std::net::TcpStream`] implements in the binary and an in-memory
//! pair implements in a test, so no unit test opens a connection.

use std::io::{Read, Write};

pub mod ftp;
pub mod http;

/// A byte stream to the console.
pub trait Wire: Read + Write {}

impl<T: Read + Write> Wire for T {}

/// An FTP data connection: a [`Wire`] whose sending half can close while
/// the stream stays borrowed, so the server sees the end of a stored
/// file even when the caller keeps the stream.
pub trait DataConnection: Wire {
    /// Close the sending half.
    ///
    /// # Errors
    ///
    /// The stream's own refusal.
    fn close_write(&mut self) -> std::io::Result<()>;
}

impl DataConnection for std::net::TcpStream {
    fn close_write(&mut self) -> std::io::Result<()> {
        self.shutdown(std::net::Shutdown::Write)
    }
}

impl<T: DataConnection + ?Sized> DataConnection for &mut T {
    fn close_write(&mut self) -> std::io::Result<()> {
        (**self).close_write()
    }
}

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

/// Why an exchange with the console failed.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// The console's address did not resolve, or did not accept a
    /// connection: the console is off, or not where the host names it.
    #[error("{operation} {address}: {source}")]
    Unreachable {
        /// `resolve` or `connect to`.
        operation: &'static str,
        /// The host and port.
        address: String,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The stream refused a read or a write.
    #[error("{operation}: {source}")]
    Io {
        /// What the client was doing.
        operation: &'static str,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A value the client would put on the wire could split or smuggle a
    /// request: it is empty, or holds whitespace or a control character.
    #[error("{field} value {value:?} is empty or holds whitespace or a control character")]
    BadArgument {
        /// Which value.
        field: &'static str,
        /// The value as given.
        value: String,
    },
    /// An HTTP request path that does not start with `/`.
    #[error("HTTP request path {0:?} does not start with /")]
    RelativePath(String),
    /// The HTTP response never ends its headers.
    #[error("HTTP response headers never end")]
    NoHeaderEnd,
    /// The HTTP status line is not `HTTP/1.x NNN ...`.
    #[error("HTTP status line {0:?} is malformed")]
    BadStatusLine(String),
    /// An HTTP header line with no colon, or with a bare CR or LF.
    #[error("HTTP header line {0:?} has no colon or holds a bare CR or LF")]
    BadHeader(String),
    /// A `Content-Length` that is not a byte count.
    #[error("HTTP Content-Length {0:?} is not a byte count")]
    BadContentLength(String),
    /// The HTTP body is not the length its header states.
    #[error("HTTP body holds {found} bytes, Content-Length states {expected}")]
    BodyLength {
        /// Bytes received.
        found: usize,
        /// Bytes the header states.
        expected: usize,
    },
    /// An FTP reply line that does not open with a three-digit code.
    #[error("FTP reply line {0:?} does not open with a three-digit code")]
    BadReplyLine(String),
    /// The FTP control connection closed before a reply ended.
    #[error("FTP control connection closed before the reply ended")]
    ReplyTruncated,
    /// An FTP reply code the command does not accept.
    #[error("FTP {command} answered {code}: {text}")]
    UnexpectedReply {
        /// The command as sent.
        command: String,
        /// The reply code.
        code: u16,
        /// The reply text.
        text: String,
    },
    /// A PASV reply with no address six-tuple.
    #[error("FTP PASV reply {0:?} holds no address six-tuple")]
    BadPasv(String),
    /// An earlier exchange on the FTP session failed with its reply still
    /// unread, so the next reply would answer the wrong command.
    #[error("FTP session refuses {0}: an earlier exchange failed with its reply unread")]
    Desynced(String),
    /// An HTTP status the request does not accept.
    #[error("HTTP GET {path} answered {status}")]
    UnexpectedStatus {
        /// The request path.
        path: String,
        /// The status code.
        status: u16,
    },
}

/// The refusal for a value that could split a request line.
fn check_argument(field: &'static str, value: &str) -> Result<(), TransportError> {
    if value.is_empty() || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(TransportError::BadArgument {
            field,
            value: value.to_string(),
        });
    }
    Ok(())
}

fn io_error(operation: &'static str) -> impl FnOnce(std::io::Error) -> TransportError {
    move |source| TransportError::Io { operation, source }
}
