//! The HTTP/1.0 client: one `GET` per connection, `Connection: close`,
//! and a pure response parser. A 404 is a value (the result file is
//! absent), not an error.

use super::{check_argument, io_error, TransportError, Wire};
use crate::transcript::Transcript;

/// The blank line that ends the headers.
const HEADER_END: &[u8] = b"\r\n\r\n";

/// One HTTP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// The status code.
    pub status: u16,
    /// Each header as `(name, value)`, trimmed, in arrival order.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// The first header named `name`, compared without case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Whether the server has no file at the path.
    pub fn is_not_found(&self) -> bool {
        self.status == 404
    }
}

/// The request bytes for `GET path` to `host`.
///
/// # Errors
///
/// [`TransportError::BadArgument`] for a host or path that is empty or
/// holds whitespace or a control character, and
/// [`TransportError::RelativePath`] for a path not starting with `/`.
pub fn request_bytes(host: &str, path: &str) -> Result<Vec<u8>, TransportError> {
    check_argument("HTTP host", host)?;
    check_argument("HTTP path", path)?;
    if !path.starts_with('/') {
        return Err(TransportError::RelativePath(path.to_string()));
    }
    Ok(format!("GET {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n").into_bytes())
}

/// Send `GET path` over `wire`, read to the end of the stream, and
/// parse the response. The transcript gets the request and the status.
///
/// # Errors
///
/// A [`request_bytes`] refusal, [`TransportError::Io`] from the stream,
/// or a [`parse_response`] refusal.
pub fn get<W: Wire>(
    wire: &mut W,
    host: &str,
    path: &str,
    transcript: &mut Transcript,
) -> Result<HttpResponse, TransportError> {
    let request = request_bytes(host, path)?;
    transcript.request(format!("GET {path}"));
    wire.write_all(&request).map_err(io_error("HTTP write"))?;
    wire.flush().map_err(io_error("HTTP write"))?;
    let mut raw = Vec::new();
    wire.read_to_end(&mut raw).map_err(io_error("HTTP read"))?;
    let response = parse_response(&raw)?;
    transcript.reply(format!(
        "{} ({} bytes)",
        response.status,
        response.body.len()
    ));
    Ok(response)
}

/// Parse a whole response as the server closed the stream on it.
///
/// # Errors
///
/// [`TransportError::NoHeaderEnd`], [`TransportError::BadStatusLine`],
/// [`TransportError::BadHeader`], [`TransportError::BadContentLength`],
/// and [`TransportError::BodyLength`] when a stated length disagrees
/// with the body.
pub fn parse_response(raw: &[u8]) -> Result<HttpResponse, TransportError> {
    let end = raw
        .windows(HEADER_END.len())
        .position(|window| window == HEADER_END)
        .ok_or(TransportError::NoHeaderEnd)?;
    let head = String::from_utf8_lossy(&raw[..end]);
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status = parse_status_line(status_line)?;
    let mut headers = Vec::new();
    for line in lines {
        // RFC 1945 section 2.2: CR LF ends every header line. A bare CR or
        // LF means the headers did not end where the search found the
        // blank line, which was inside the body.
        if line.contains(['\r', '\n']) {
            return Err(TransportError::BadHeader(line.to_string()));
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| TransportError::BadHeader(line.to_string()))?;
        headers.push((name.trim().to_string(), value.trim().to_string()));
    }
    let body = raw[end + HEADER_END.len()..].to_vec();
    let response = HttpResponse {
        status,
        headers,
        body,
    };
    if let Some(stated) = response.header("Content-Length") {
        let expected: usize = stated
            .parse()
            .map_err(|_| TransportError::BadContentLength(stated.to_string()))?;
        if response.body.len() != expected {
            return Err(TransportError::BodyLength {
                found: response.body.len(),
                expected,
            });
        }
    }
    Ok(response)
}

fn parse_status_line(line: &str) -> Result<u16, TransportError> {
    let bad = || TransportError::BadStatusLine(line.to_string());
    if line.contains(['\r', '\n']) {
        return Err(bad());
    }
    let mut parts = line.splitn(3, ' ');
    let version = parts.next().unwrap_or_default();
    if !version.starts_with("HTTP/1.") {
        return Err(bad());
    }
    let code = parts.next().unwrap_or_default();
    if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    code.parse().map_err(|_| bad())
}

#[cfg(test)]
#[path = "tests/http_tests.rs"]
mod tests;
