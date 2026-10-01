//! The FTP client: anonymous login, binary type, passive data
//! connections, and the store, list, delete and remove-directory
//! commands deployment and cleanup need, over a pure reply parser that
//! handles multi-line replies.
//!
//! A data connection is the caller's to open: [`FtpSession::pasv`]
//! returns the address, and [`FtpSession::stor`] / [`FtpSession::nlst`]
//! take the connected stream.

use std::io::{ErrorKind, Read};
use std::net::{Ipv4Addr, SocketAddrV4};

use super::{check_argument, io_error, DataConnection, TransportError, Wire};
use crate::transcript::Transcript;

/// One FTP reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FtpReply {
    /// The three-digit code.
    pub code: u16,
    /// Each line's text after the code and its separator.
    pub lines: Vec<String>,
}

impl FtpReply {
    /// The lines joined into one.
    pub fn text(&self) -> String {
        self.lines.join(" / ")
    }
}

/// The reply at the start of `text`, and the byte count it spans.
///
/// `Ok(None)` means the text ends before the reply does. A single-line
/// reply is `NNN text`; a multi-line one opens with `NNN-text` and ends
/// at the first later line that opens with the same `NNN` and a space.
///
/// # Errors
///
/// [`TransportError::BadReplyLine`] when the first line does not open
/// with a three-digit code and a space or hyphen.
pub fn parse_reply(text: &str) -> Result<Option<(FtpReply, usize)>, TransportError> {
    let mut consumed = 0;
    let mut code = None;
    let mut lines = Vec::new();
    let mut multi = false;
    for line in text.split_inclusive('\n') {
        if !line.ends_with('\n') {
            return Ok(None);
        }
        consumed += line.len();
        let line = line.trim_end_matches(['\r', '\n']);
        match code {
            None => {
                let (parsed, separator, rest) = split_code(line)
                    .ok_or_else(|| TransportError::BadReplyLine(line.to_string()))?;
                code = Some(parsed);
                multi = separator == '-';
                lines.push(rest.to_string());
                if !multi {
                    break;
                }
            }
            Some(open) => match split_code(line) {
                Some((parsed, ' ', rest)) if parsed == open => {
                    lines.push(rest.to_string());
                    multi = false;
                    break;
                }
                _ => lines.push(line.trim_start().to_string()),
            },
        }
    }
    match code {
        Some(code) if !multi => Ok(Some((FtpReply { code, lines }, consumed))),
        _ => Ok(None),
    }
}

/// `NNN` and its separator (` ` or `-`) and the rest, when the line
/// opens that way. A bare `NNN` reads as `NNN ` with no text.
fn split_code(line: &str) -> Option<(u16, char, &str)> {
    let digits = line.get(..3)?;
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let code = digits.parse().ok()?;
    match line[3..].chars().next() {
        None => Some((code, ' ', "")),
        Some(separator @ (' ' | '-')) => Some((code, separator, &line[4..])),
        Some(_) => None,
    }
}

/// Read exactly one reply from `wire`, a byte at a time so the read
/// consumes nothing past the reply.
///
/// # Errors
///
/// [`TransportError::Io`], [`TransportError::ReplyTruncated`] when the
/// stream ends first, and a [`parse_reply`] refusal.
pub fn read_reply<R: Read>(wire: &mut R) -> Result<FtpReply, TransportError> {
    let mut raw = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        // `read_exact` retries an `Interrupted` read, which a bare `read`
        // hands back as a failure, and names the end of the stream.
        match wire.read_exact(&mut byte) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                return Err(TransportError::ReplyTruncated);
            }
            Err(e) => return Err(io_error("FTP read")(e)),
        }
        raw.push(byte[0]);
        if byte[0] == b'\n' {
            if let Some((reply, _)) = parse_reply(&String::from_utf8_lossy(&raw))? {
                return Ok(reply);
            }
        }
    }
}

/// The address in a `227` reply's `(h1,h2,h3,h4,p1,p2)`.
///
/// # Errors
///
/// [`TransportError::BadPasv`] when the text holds no such tuple.
pub fn parse_pasv(text: &str) -> Result<SocketAddrV4, TransportError> {
    let bad = || TransportError::BadPasv(text.to_string());
    let open = text.find('(').ok_or_else(bad)?;
    let close = text[open..].find(')').ok_or_else(bad)? + open;
    let fields: Vec<u8> = text[open + 1..close]
        .split(',')
        .map(|field| field.trim().parse::<u8>())
        .collect::<Result<_, _>>()
        .map_err(|_| bad())?;
    let [a, b, c, d, high, low] = fields[..] else {
        return Err(bad());
    };
    Ok(SocketAddrV4::new(
        Ipv4Addr::new(a, b, c, d),
        u16::from(high) << 8 | u16::from(low),
    ))
}

/// Whether a reply ends its command's exchange or a later one follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// A preliminary `1xx` reply; the closing reply is still owed.
    Preliminary,
    /// The reply that ends the exchange.
    Closing,
}

/// A logged-in control connection.
///
/// A command whose exchange fails before its closing reply is read
/// leaves that reply on the connection. The session then refuses every
/// later command with [`TransportError::Desynced`] rather than read the
/// stale reply as the next command's answer. A reply the command does
/// not accept still ends the exchange, so the session stays usable,
/// unless it is a preliminary `1xx` reply.
#[derive(Debug)]
pub struct FtpSession<W: Wire> {
    control: W,
    reply_owed: bool,
}

impl<W: Wire> FtpSession<W> {
    /// Take the control connection and read the server's greeting.
    ///
    /// # Errors
    ///
    /// Any [`read_reply`] error, or [`TransportError::UnexpectedReply`]
    /// when the greeting is not `220`.
    pub fn open(control: W, transcript: &mut Transcript) -> Result<Self, TransportError> {
        let mut session = Self {
            control,
            reply_owed: true,
        };
        session.expect("greeting", &[220], Step::Closing, transcript)?;
        Ok(session)
    }

    fn send(
        &mut self,
        line: &str,
        note: &str,
        transcript: &mut Transcript,
    ) -> Result<(), TransportError> {
        if self.reply_owed {
            return Err(TransportError::Desynced(line.to_string()));
        }
        self.reply_owed = true;
        transcript.request(format!("{line}{note}"));
        self.control
            .write_all(format!("{line}\r\n").as_bytes())
            .map_err(io_error("FTP write"))?;
        self.control.flush().map_err(io_error("FTP write"))
    }

    fn expect(
        &mut self,
        command: &str,
        codes: &[u16],
        step: Step,
        transcript: &mut Transcript,
    ) -> Result<FtpReply, TransportError> {
        let reply = read_reply(&mut self.control)?;
        transcript.reply(format!("{} {}", reply.code, reply.text()));
        if codes.contains(&reply.code) {
            if step == Step::Closing {
                self.reply_owed = false;
            }
            Ok(reply)
        } else {
            // RFC 959 section 4.2: a 1xx reply is preliminary, so another
            // reply still follows it on the control connection.
            self.reply_owed = (100..200).contains(&reply.code);
            Err(TransportError::UnexpectedReply {
                command: command.to_string(),
                code: reply.code,
                text: reply.text(),
            })
        }
    }

    fn command(
        &mut self,
        line: &str,
        codes: &[u16],
        transcript: &mut Transcript,
    ) -> Result<FtpReply, TransportError> {
        self.send(line, "", transcript)?;
        self.expect(line, codes, Step::Closing, transcript)
    }

    /// `USER anonymous`, then `PASS` when the server asks for one.
    ///
    /// # Errors
    ///
    /// Any exchange error, or a reply other than `230` / `331`.
    pub fn login_anonymous(&mut self, transcript: &mut Transcript) -> Result<(), TransportError> {
        let user = self.command("USER anonymous", &[230, 331], transcript)?;
        if user.code == 331 {
            self.command("PASS anonymous@", &[230], transcript)?;
        }
        Ok(())
    }

    /// `TYPE I`, so the server rewrites no byte of a stored file.
    ///
    /// # Errors
    ///
    /// Any exchange error, or a reply other than `200`.
    pub fn type_binary(&mut self, transcript: &mut Transcript) -> Result<(), TransportError> {
        self.command("TYPE I", &[200], transcript).map(drop)
    }

    /// `PASV`, returning the address the next data connection opens to.
    ///
    /// # Errors
    ///
    /// Any exchange error, a reply other than `227`, or a
    /// [`parse_pasv`] refusal.
    pub fn pasv(&mut self, transcript: &mut Transcript) -> Result<SocketAddrV4, TransportError> {
        let reply = self.command("PASV", &[227], transcript)?;
        parse_pasv(&reply.text())
    }

    /// `MKD path`.
    ///
    /// # Errors
    ///
    /// A path that could split the command, any exchange error, or a
    /// reply other than `257`.
    pub fn mkd(&mut self, path: &str, transcript: &mut Transcript) -> Result<(), TransportError> {
        check_argument("FTP path", path)?;
        self.command(&format!("MKD {path}"), &[257], transcript)
            .map(drop)
    }

    /// `STOR path`, then `bytes` over `data`, whose sending half this
    /// call closes so the server sees the end of the file.
    ///
    /// # Errors
    ///
    /// A path that could split the command, any exchange error, a reply
    /// other than `125` / `150` before the transfer or `226` / `250`
    /// after it, or [`TransportError::Io`] on the data connection.
    pub fn stor<D: DataConnection>(
        &mut self,
        path: &str,
        mut data: D,
        bytes: &[u8],
        transcript: &mut Transcript,
    ) -> Result<(), TransportError> {
        check_argument("FTP path", path)?;
        let line = format!("STOR {path}");
        self.send(&line, &format!(" ({} bytes)", bytes.len()), transcript)?;
        self.expect(&line, &[125, 150], Step::Preliminary, transcript)?;
        data.write_all(bytes).map_err(io_error("FTP data write"))?;
        data.flush().map_err(io_error("FTP data write"))?;
        data.close_write().map_err(io_error("FTP data close"))?;
        drop(data);
        self.expect(&line, &[226, 250], Step::Closing, transcript)
            .map(drop)
    }

    /// `NLST path`, reading the names over `data` to its end.
    ///
    /// # Errors
    ///
    /// A path that could split the command, any exchange error, a reply
    /// other than `125` / `150` before the transfer or `226` / `250`
    /// after it, or [`TransportError::Io`] on the data connection.
    pub fn nlst<D: DataConnection>(
        &mut self,
        path: &str,
        mut data: D,
        transcript: &mut Transcript,
    ) -> Result<Vec<String>, TransportError> {
        check_argument("FTP path", path)?;
        let line = format!("NLST {path}");
        self.send(&line, "", transcript)?;
        self.expect(&line, &[125, 150], Step::Preliminary, transcript)?;
        let mut raw = Vec::new();
        data.read_to_end(&mut raw)
            .map_err(io_error("FTP data read"))?;
        drop(data);
        self.expect(&line, &[226, 250], Step::Closing, transcript)?;
        Ok(String::from_utf8_lossy(&raw)
            .lines()
            .map(|name| name.trim_end_matches('\r').to_string())
            .filter(|name| !name.is_empty())
            .collect())
    }

    /// `DELE path`.
    ///
    /// # Errors
    ///
    /// A path that could split the command, any exchange error, or a
    /// reply other than `250`.
    pub fn dele(&mut self, path: &str, transcript: &mut Transcript) -> Result<(), TransportError> {
        check_argument("FTP path", path)?;
        self.command(&format!("DELE {path}"), &[250], transcript)
            .map(drop)
    }

    /// `RMD path`.
    ///
    /// # Errors
    ///
    /// A path that could split the command, any exchange error, or a
    /// reply other than `250`.
    pub fn rmd(&mut self, path: &str, transcript: &mut Transcript) -> Result<(), TransportError> {
        check_argument("FTP path", path)?;
        self.command(&format!("RMD {path}"), &[250], transcript)
            .map(drop)
    }

    /// `QUIT`, returning the control connection.
    ///
    /// # Errors
    ///
    /// Any exchange error, or a reply other than `221`.
    pub fn quit(mut self, transcript: &mut Transcript) -> Result<W, TransportError> {
        self.command("QUIT", &[221], transcript)?;
        Ok(self.control)
    }
}

#[cfg(test)]
#[path = "tests/ftp_tests.rs"]
mod tests;
