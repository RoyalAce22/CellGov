//! The numbered record of every request, reply and decision of one
//! run. A redactor masks console identifiers and the console's address
//! before the transcript stores a line.
//!
//! A line opens with `#`, its four-digit sequence number and a marker:
//! `>` for a request, `<` for a reply, `=` for a decision. The text
//! follows on the same line.

use std::path::Path;

use cellgov_observation::hardware_capture::TRANSCRIPT_FILE;

/// The words an identifier follows on the console's status page.
const ID_KEYWORDS: [&str; 2] = ["IDPS", "PSID"];
/// The shortest hex run after a keyword that reads as an identifier.
const ID_HEX_DIGITS: usize = 32;
/// How many characters past a keyword the identifier may sit, to skip
/// the markup between a label and its value.
const ID_SEARCH_WINDOW: usize = 200;
/// What an identifier becomes.
pub const MASKED_ID: &str = "<id>";
/// What a MAC address becomes.
pub const MASKED_MAC: &str = "<mac>";
/// What the console's host name or IPv4 address becomes.
pub const MASKED_HOST: &str = "<host>";

/// The fields of an FTP host-port argument, `h1,h2,h3,h4,p1,p2`, in
/// RFC 959 section 4.1.2. The first four fields are the host.
const HOST_PORT_FIELDS: usize = 6;
/// The fields of an IPv4 address that name the host.
const HOST_FIELDS: usize = 4;

/// One masked span: its start, its end and its mask.
type Span = (usize, usize, &'static str);

/// One run's lines, numbered from 1 in the order they arrive.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Transcript {
    lines: Vec<String>,
    host: Option<String>,
}

impl Transcript {
    /// An empty transcript.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty transcript that also masks `host`, the name or address
    /// the runner reaches the console by.
    pub fn for_host(host: &str) -> Self {
        let host = host.trim();
        Self {
            lines: Vec::new(),
            host: (!host.is_empty()).then(|| host.to_string()),
        }
    }

    /// Something the runner sent.
    pub fn request(&mut self, text: impl AsRef<str>) {
        self.push('>', text.as_ref());
    }

    /// Something the console answered.
    pub fn reply(&mut self, text: impl AsRef<str>) {
        self.push('<', text.as_ref());
    }

    /// Something the runner decided.
    pub fn decision(&mut self, text: impl AsRef<str>) {
        self.push('=', text.as_ref());
    }

    fn push(&mut self, marker: char, text: &str) {
        let number = self.lines.len() + 1;
        let flat: String = text
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let masked = match &self.host {
            Some(host) => redact_with_host(&flat, host),
            None => redact(&flat),
        };
        self.lines.push(format!("#{number:04} {marker} {masked}"));
    }

    /// The stored lines.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The lines as the file holds them, one per line.
    pub fn render(&self) -> String {
        let mut text = self.lines.join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        text
    }

    /// Write the transcript into `dir`.
    ///
    /// # Errors
    ///
    /// The write's I/O error.
    pub fn write(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::write(dir.join(TRANSCRIPT_FILE), self.render())
    }
}

/// `text` with these masked:
///
/// - every identifier that follows `IDPS` or `PSID`;
/// - every MAC-shaped token;
/// - every dotted IPv4 address;
/// - the host of every FTP host-port argument, which a `227` reply to
///   `PASV` and a `PORT` command carry as `h1,h2,h3,h4,p1,p2`.
pub fn redact(text: &str) -> String {
    redact_spans(text, None)
}

/// [`redact`], and also every token that is `host`, the name or address
/// the runner reaches the console by.
pub fn redact_with_host(text: &str, host: &str) -> String {
    let host: Vec<char> = host.trim().chars().collect();
    redact_spans(text, (!host.is_empty()).then_some(host.as_slice()))
}

fn redact_spans(text: &str, host: Option<&[char]>) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut spans = id_spans(&chars);
    spans.extend(mac_spans(&chars));
    spans.extend(dotted_host_spans(&chars));
    spans.extend(host_port_spans(&chars));
    if let Some(host) = host {
        spans.extend(named_host_spans(&chars, host));
    }
    spans.sort_by_key(|(start, _, _)| *start);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (start, end, mask) in spans {
        if start < at {
            continue;
        }
        out.extend(&chars[at..start]);
        out.push_str(mask);
        at = end;
    }
    out.extend(&chars[at..]);
    out
}

/// Each run of at least [`ID_HEX_DIGITS`] hex digits that starts within
/// [`ID_SEARCH_WINDOW`] characters after a keyword. Every such run is
/// masked, not only the first: a table with the labels in one row and
/// the values in the next puts both values after the `PSID` label.
fn id_spans(chars: &[char]) -> Vec<(usize, usize, &'static str)> {
    let mut spans = Vec::new();
    for keyword in ID_KEYWORDS {
        let key: Vec<char> = keyword.chars().collect();
        let mut at = 0;
        while at + key.len() <= chars.len() {
            let matched = chars[at..at + key.len()]
                .iter()
                .zip(&key)
                .all(|(c, k)| c.eq_ignore_ascii_case(k));
            if !matched {
                at += 1;
                continue;
            }
            let window_end = (at + key.len() + ID_SEARCH_WINDOW).min(chars.len());
            let mut run_start = at + key.len();
            while run_start < window_end {
                let run_end = hex_run_end(chars, run_start);
                if run_end - run_start >= ID_HEX_DIGITS {
                    spans.push((run_start, run_end, MASKED_ID));
                }
                run_start = run_end.max(run_start + 1);
            }
            at += key.len();
        }
    }
    spans
}

fn hex_run_end(chars: &[char], start: usize) -> usize {
    chars[start..]
        .iter()
        .position(|c| !c.is_ascii_hexdigit())
        .map_or(chars.len(), |offset| start + offset)
}

/// Each `hh:hh:hh:hh:hh:hh` or `hh-hh-hh-hh-hh-hh` token that no
/// alphanumeric character touches.
fn mac_spans(chars: &[char]) -> Vec<(usize, usize, &'static str)> {
    const LEN: usize = 17;
    let mut spans = Vec::new();
    let mut at = 0;
    while at + LEN <= chars.len() {
        let token = &chars[at..at + LEN];
        let separator = token[2];
        let shaped = (separator == ':' || separator == '-')
            && token.iter().enumerate().all(|(i, c)| {
                if i % 3 == 2 {
                    *c == separator
                } else {
                    c.is_ascii_hexdigit()
                }
            });
        let bounded = (at == 0 || !chars[at - 1].is_ascii_alphanumeric())
            && chars
                .get(at + LEN)
                .is_none_or(|c| !c.is_ascii_alphanumeric());
        if shaped && bounded {
            spans.push((at, at + LEN, MASKED_MAC));
            at += LEN;
        } else {
            at += 1;
        }
    }
    spans
}

/// The end of the decimal field from 0 to 255 that starts at `start`,
/// when one does. A fourth digit makes the field too long.
fn octet_end(chars: &[char], start: usize) -> Option<usize> {
    let end = chars[start..]
        .iter()
        .take(4)
        .position(|c| !c.is_ascii_digit())
        .map_or((start + 4).min(chars.len()), |offset| start + offset);
    let digits = end - start;
    if !(1..=3).contains(&digits) {
        return None;
    }
    let value: String = chars[start..end].iter().collect();
    value.parse::<u8>().ok().map(|_| end)
}

/// The end of `count` fields from 0 to 255 that start at `start`, with
/// each field after the first opening past a `separator` that
/// `skip_spaces` lets spaces surround. Each field's end is returned.
fn octet_run(
    chars: &[char],
    start: usize,
    count: usize,
    separator: char,
    skip_spaces: bool,
) -> Option<Vec<usize>> {
    let skip = |mut at: usize| {
        while skip_spaces && chars.get(at) == Some(&' ') {
            at += 1;
        }
        at
    };
    let mut ends = Vec::with_capacity(count);
    let mut at = start;
    for field in 0..count {
        if field > 0 {
            at = skip(at);
            if chars.get(at) != Some(&separator) {
                return None;
            }
            at = skip(at + 1);
        }
        at = octet_end(chars, at)?;
        ends.push(at);
    }
    Some(ends)
}

/// Each dotted IPv4 address that no alphanumeric character, dot or
/// further dotted field touches. A port after a colon stays.
fn dotted_host_spans(chars: &[char]) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let opens = chars[at].is_ascii_digit()
            && (at == 0 || !(chars[at - 1].is_ascii_alphanumeric() || chars[at - 1] == '.'));
        let end = opens
            .then(|| octet_run(chars, at, HOST_FIELDS, '.', false))
            .flatten()
            .and_then(|ends| ends.last().copied());
        let bounded = end.is_some_and(|end| match chars.get(end) {
            None => true,
            Some('.') => !chars.get(end + 1).is_some_and(char::is_ascii_alphanumeric),
            Some(c) => !c.is_ascii_alphanumeric(),
        });
        match end {
            Some(end) if bounded => {
                spans.push((at, end, MASKED_HOST));
                at = end;
            }
            _ => at += 1,
        }
    }
    spans
}

/// The host fields of each FTP host-port argument: six comma-separated
/// fields from 0 to 255 that no further digit or comma touches.
fn host_port_spans(chars: &[char]) -> Vec<Span> {
    let touches = |c: Option<&char>| c.is_some_and(|c| c.is_ascii_digit() || *c == ',');
    let mut spans = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let before = chars[..at].iter().rev().find(|c| **c != ' ');
        let run = (chars[at].is_ascii_digit() && !touches(before))
            .then(|| octet_run(chars, at, HOST_PORT_FIELDS, ',', true))
            .flatten();
        let after = run.as_ref().and_then(|ends| {
            let last = *ends.last()?;
            chars[last..].iter().find(|c| **c != ' ')
        });
        match run {
            Some(ends) if !touches(after) => {
                let host_end = ends[HOST_FIELDS - 1];
                spans.push((at, host_end, MASKED_HOST));
                at = ends[HOST_PORT_FIELDS - 1];
            }
            _ => at += 1,
        }
    }
    spans
}

/// Each token that is `host`, compared without regard to ASCII case,
/// that no character of a longer host name touches.
fn named_host_spans(chars: &[char], host: &[char]) -> Vec<Span> {
    let name_char = |c: &char| c.is_ascii_alphanumeric() || *c == '-' || *c == '_';
    let mut spans = Vec::new();
    let mut at = 0;
    while at + host.len() <= chars.len() {
        let matched = chars[at..at + host.len()]
            .iter()
            .zip(host)
            .all(|(c, h)| c.eq_ignore_ascii_case(h));
        let end = at + host.len();
        let bounded = (at == 0 || !(name_char(&chars[at - 1]) || chars[at - 1] == '.'))
            && match chars.get(end) {
                None => true,
                Some('.') => !chars.get(end + 1).is_some_and(name_char),
                Some(c) => !name_char(c),
            };
        if matched && bounded {
            spans.push((at, end, MASKED_HOST));
            at = end;
        } else {
            at += 1;
        }
    }
    spans
}

#[cfg(test)]
#[path = "tests/transcript_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/transcript_host_tests.rs"]
mod host_tests;
