//! The numbered record of every request, reply and decision of one
//! run. A redactor masks console identifiers before the transcript
//! stores a line.
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

/// One run's lines, numbered from 1 in the order they arrive.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Transcript {
    lines: Vec<String>,
}

impl Transcript {
    /// An empty transcript.
    pub fn new() -> Self {
        Self::default()
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
        self.lines
            .push(format!("#{number:04} {marker} {}", redact(&flat)));
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

/// `text` with every identifier that follows `IDPS` or `PSID` and every
/// MAC-shaped token masked.
pub fn redact(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut spans = id_spans(&chars);
    spans.extend(mac_spans(&chars));
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

#[cfg(test)]
#[path = "tests/transcript_tests.rs"]
mod tests;
