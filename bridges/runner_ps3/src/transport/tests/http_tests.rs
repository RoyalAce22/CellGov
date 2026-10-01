//! The HTTP client over an in-memory stream: the request bytes, the
//! response parser's refusals, a 404 as a value, and the transcript.

use std::io::{Cursor, Read, Write};

use super::*;

/// A stream whose reads come from `input` and whose writes land in
/// `output`.
struct MemoryWire {
    input: Cursor<Vec<u8>>,
    output: Vec<u8>,
}

impl MemoryWire {
    fn answering(response: &[u8]) -> Self {
        Self {
            input: Cursor::new(response.to_vec()),
            output: Vec::new(),
        }
    }
}

impl Read for MemoryWire {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.input.read(buf)
    }
}

impl Write for MemoryWire {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.output.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_get_sends_one_http_1_0_request_that_closes_and_parses_the_answer() {
    let mut wire = MemoryWire::answering(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 5\r\n\r\nhello",
    );
    let mut transcript = Transcript::new();
    let response = get(&mut wire, "10.77.0.2", "/cpursx.ps3", &mut transcript).expect("200");
    assert_eq!(
        wire.output,
        b"GET /cpursx.ps3 HTTP/1.0\r\nHost: 10.77.0.2\r\nConnection: close\r\n\r\n"
    );
    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-type"), Some("text/html"));
    assert_eq!(response.body, b"hello");
    assert_eq!(
        transcript.lines(),
        ["#0001 > GET /cpursx.ps3", "#0002 < 200 (5 bytes)"]
    );
}

#[test]
fn a_404_is_a_value_not_an_error() {
    let response = parse_response(b"HTTP/1.0 404 Not Found\r\n\r\n").expect("a 404 still parses");
    assert_eq!(response.status, 404);
    assert!(response.is_not_found());
    assert!(response.body.is_empty());
}

#[test]
fn a_body_with_no_length_header_runs_to_the_end_of_the_stream() {
    let response = parse_response(b"HTTP/1.0 200 OK\r\n\r\nCGOV\x00\x00").expect("parses");
    assert_eq!(response.body, b"CGOV\x00\x00");
}

#[test]
fn a_body_of_another_length_than_its_header_states_is_refused_either_way() {
    for (raw, found) in [
        (&b"HTTP/1.0 200 OK\r\nContent-Length: 6\r\n\r\nhello"[..], 5),
        (&b"HTTP/1.0 200 OK\r\nContent-Length: 4\r\n\r\nhello"[..], 5),
    ] {
        let err = parse_response(raw).expect_err("refused");
        assert!(
            matches!(err, TransportError::BodyLength { found: f, .. } if f == found),
            "{err:?}"
        );
    }
}

#[test]
fn the_parser_names_each_malformed_part() {
    assert!(matches!(
        parse_response(b"HTTP/1.0 200 OK\r\nno end"),
        Err(TransportError::NoHeaderEnd)
    ));
    for line in [
        "HTTP/2 200 OK",
        "HTTP/1.0 20 OK",
        "HTTP/1.0 2x0 OK",
        "ICY 200 OK",
        "",
    ] {
        let raw = format!("{line}\r\n\r\n");
        assert!(
            matches!(parse_response(raw.as_bytes()), Err(TransportError::BadStatusLine(l)) if l == line),
            "{line:?}"
        );
    }
    assert!(matches!(
        parse_response(b"HTTP/1.0 200 OK\r\nno colon here\r\n\r\n"),
        Err(TransportError::BadHeader(l)) if l == "no colon here"
    ));
    assert!(matches!(
        parse_response(b"HTTP/1.0 200 OK\r\nContent-Length: many\r\n\r\n"),
        Err(TransportError::BadContentLength(v)) if v == "many"
    ));
}

#[test]
fn bare_line_feed_headers_do_not_end_at_a_blank_line_inside_the_body() {
    assert!(matches!(
        parse_response(b"HTTP/1.0 200 OK\nContent-Length: 9\n\na:b\r\n\r\nrest"),
        Err(TransportError::BadStatusLine(_))
    ));
    assert!(matches!(
        parse_response(b"HTTP/1.0 200 OK\r\nServer: x\nContent-Length: 4\r\n\r\nrest"),
        Err(TransportError::BadHeader(_))
    ));
}

#[test]
fn a_path_or_host_that_could_split_the_request_is_refused_before_anything_is_sent() {
    for (host, path) in [
        ("10.77.0.2", "/a b"),
        ("10.77.0.2", "/a\r\nHost: x"),
        ("10.77.0.2", ""),
        ("10.77.0.2 x", "/a"),
        ("", "/a"),
    ] {
        let mut wire = MemoryWire::answering(b"HTTP/1.0 200 OK\r\n\r\n");
        let mut transcript = Transcript::new();
        let err = get(&mut wire, host, path, &mut transcript).expect_err("refused");
        assert!(matches!(err, TransportError::BadArgument { .. }), "{err:?}");
        assert!(wire.output.is_empty(), "{host:?} {path:?} reached the wire");
        assert!(transcript.lines().is_empty());
    }
    let err = request_bytes("10.77.0.2", "cpursx.ps3").expect_err("relative");
    assert!(matches!(err, TransportError::RelativePath(p) if p == "cpursx.ps3"));
}
