//! The crash ledger's peek ranges as a stop list.
//!
//! [`LEDGER`] records, in prose, every call and read that took the
//! reference console down. [`STOP_LIST`] beside it holds the ranges of
//! the ledger's peek rows in a form the runner reads. The runner loads
//! both before it waits for the console and refuses to use the console
//! when they disagree. The runner refuses a peek whose page overlaps a
//! listed range before the peek waits.
//!
//! A peek row of the ledger names its space as `` `peek.lv1` `` or
//! `` `peek.lv2` ``, and its range as `of 0x<first> to 0x<last>`, both
//! ends inside the range.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::peek::Space;

/// The prose ledger, under the workspace root.
pub const LEDGER: &str = "tests/micro/console_crashes.md";

/// The stop list, under the workspace root.
pub const STOP_LIST: &str = "tests/micro/console_crashes.tsv";

/// The stop list's first line.
pub const HEADER: &str = "space\tfirst\tlast\tstatus";

/// How sure the ledger is that a range takes the console down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// A run that sent it crashed, with other suspects.
    Suspected,
    /// A run whose only new read was this one crashed.
    Confirmed,
}

impl Status {
    fn from_name(word: &str) -> Option<Self> {
        match word {
            "suspected" => Some(Self::Suspected),
            "confirmed" => Some(Self::Confirmed),
            _ => None,
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Suspected => "suspected",
            Self::Confirmed => "confirmed",
        })
    }
}

/// One listed range: every address from `first` to `last` in `space`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopRange {
    /// The memory.
    pub space: Space,
    /// The range's first address.
    pub first: u64,
    /// The range's last address.
    pub last: u64,
    /// The ledger's status for the row.
    pub status: Status,
}

impl fmt::Display for StopRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} 0x{:016x} to 0x{:016x} ({})",
            self.space, self.first, self.last, self.status
        )
    }
}

/// Why the stop list did not load, or refused a peek.
#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    /// A ledger file did not read.
    #[error("read {path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A line of a ledger file does not parse.
    #[error("{file} line {line}: {reason}")]
    Malformed {
        /// [`LEDGER`] or [`STOP_LIST`].
        file: &'static str,
        /// The line, counted from 1.
        line: usize,
        /// What is wrong with it.
        reason: String,
    },
    /// The stop list does not hold exactly the ledger's peek ranges.
    #[error("{STOP_LIST} disagrees with {LEDGER}: {reason}; make the two name the same ranges")]
    Disagree {
        /// The first difference.
        reason: String,
    },
    /// The peek's page overlaps a listed range.
    #[error(
        "refused: the {space} page 0x{first:016x} to 0x{last:016x} overlaps {listed}, which \
         {LEDGER} lists as a range that took the console down; peek outside it"
    )]
    Listed {
        /// The memory.
        space: Space,
        /// The page's first address.
        first: u64,
        /// The page's last address.
        last: u64,
        /// The listed range it overlaps.
        listed: StopRange,
    },
}

/// The ledger's peek ranges, checked against its prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopList {
    ranges: Vec<StopRange>,
}

impl StopList {
    /// Read [`STOP_LIST`] and [`LEDGER`] under `workspace_root`.
    ///
    /// # Errors
    ///
    /// [`LedgerError::Io`] for a file that does not read, and the
    /// errors of [`StopList::parse`].
    pub fn load(workspace_root: &Path) -> Result<Self, LedgerError> {
        let read = |name: &str| {
            let path = workspace_root.join(name);
            std::fs::read_to_string(&path).map_err(|source| LedgerError::Io { path, source })
        };
        Self::parse(&read(STOP_LIST)?, &read(LEDGER)?)
    }

    /// The stop list `list` holds, checked against the prose `ledger`.
    ///
    /// # Errors
    ///
    /// [`LedgerError::Malformed`] for a line of either file that does
    /// not parse, and [`LedgerError::Disagree`] when the two do not name
    /// the same ranges in the same order.
    pub fn parse(list: &str, ledger: &str) -> Result<Self, LedgerError> {
        let listed = list_ranges(list)?;
        let named = ledger_ranges(ledger)?;
        if listed.len() != named.len() {
            return Err(LedgerError::Disagree {
                reason: format!(
                    "the ledger has {} peek rows and the stop list {} ranges",
                    named.len(),
                    listed.len()
                ),
            });
        }
        if let Some((index, (list_row, ledger_row))) = listed
            .iter()
            .zip(&named)
            .enumerate()
            .find(|(_, (a, b))| a != b)
        {
            return Err(LedgerError::Disagree {
                reason: format!(
                    "peek row {} is {ledger_row} in the ledger and {list_row} in the stop list",
                    index + 1
                ),
            });
        }
        Ok(Self { ranges: listed })
    }

    /// Every listed range, in the ledger's order.
    pub fn ranges(&self) -> &[StopRange] {
        &self.ranges
    }

    /// Admit a peek of the page from `first` to `last` in `space`.
    ///
    /// # Errors
    ///
    /// [`LedgerError::Listed`] naming the first listed range the page
    /// overlaps.
    pub fn admit(&self, space: Space, first: u64, last: u64) -> Result<(), LedgerError> {
        match self
            .ranges
            .iter()
            .find(|r| r.space == space && first <= r.last && r.first <= last)
        {
            Some(&listed) => Err(LedgerError::Listed {
                space,
                first,
                last,
                listed,
            }),
            None => Ok(()),
        }
    }
}

/// The value of `0x<hex>`.
fn hex(text: &str) -> Option<u64> {
    let digits = text.strip_prefix("0x")?;
    u64::from_str_radix(digits, 16).ok()
}

/// The ranges of the stop list's lines after its header.
fn list_ranges(list: &str) -> Result<Vec<StopRange>, LedgerError> {
    let malformed = |line: usize, reason: String| LedgerError::Malformed {
        file: STOP_LIST,
        line,
        reason,
    };
    let mut lines = list.lines();
    if lines.next() != Some(HEADER) {
        return Err(malformed(1, format!("the header is not {HEADER:?}")));
    }
    let mut ranges = Vec::new();
    for (index, text) in lines.enumerate() {
        let line = index + 2;
        let fields: Vec<&str> = text.split('\t').collect();
        let [space, first, last, status] = fields[..] else {
            return Err(malformed(line, format!("{} fields, not 4", fields.len())));
        };
        let space = Space::from_name(space)
            .ok_or_else(|| malformed(line, format!("space {space:?} is not lv1 or lv2")))?;
        let first =
            hex(first).ok_or_else(|| malformed(line, format!("first {first:?} is not 0x hex")))?;
        let last =
            hex(last).ok_or_else(|| malformed(line, format!("last {last:?} is not 0x hex")))?;
        let status = Status::from_name(status).ok_or_else(|| {
            malformed(
                line,
                format!("status {status:?} is not suspected or confirmed"),
            )
        })?;
        if first > last {
            return Err(malformed(line, "first is above last".to_string()));
        }
        ranges.push(StopRange {
            space,
            first,
            last,
            status,
        });
    }
    Ok(ranges)
}

/// The hex number after the first `marker` in `text`.
fn number_after(text: &str, marker: &str) -> Option<u64> {
    let rest = &text[text.find(marker)? + marker.len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_hexdigit())
        .unwrap_or(rest.len());
    u64::from_str_radix(&rest[..end], 16).ok()
}

/// The ranges of the ledger's peek rows: the table rows whose call
/// names `` `peek.lv1` `` or `` `peek.lv2` ``.
fn ledger_ranges(ledger: &str) -> Result<Vec<StopRange>, LedgerError> {
    let mut ranges = Vec::new();
    for (index, text) in ledger.lines().enumerate() {
        let malformed = |reason: String| LedgerError::Malformed {
            file: LEDGER,
            line: index + 1,
            reason,
        };
        let Some(row) = text.strip_prefix('|') else {
            continue;
        };
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        let (Some(call), Some(status)) = (cells.first(), cells.get(1)) else {
            continue;
        };
        let space = if call.contains("`peek.lv1`") {
            Space::Lv1
        } else if call.contains("`peek.lv2`") {
            Space::Lv2
        } else {
            continue;
        };
        let (Some(first), Some(last)) =
            (number_after(call, " of 0x"), number_after(call, " to 0x"))
        else {
            return Err(malformed(
                "a peek row names no range: write `of 0x<first> to 0x<last>`".to_string(),
            ));
        };
        let status = Status::from_name(status)
            .ok_or_else(|| malformed(format!("status {status:?} is not suspected or confirmed")))?;
        if first > last {
            return Err(malformed(
                "the range's first address is above its last".to_string(),
            ));
        }
        ranges.push(StopRange {
            space,
            first,
            last,
            status,
        });
    }
    Ok(ranges)
}

#[cfg(test)]
#[path = "tests/ledger_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/ledger_shadow_window_tests.rs"]
mod shadow_window_tests;

#[cfg(test)]
#[path = "tests/ledger_large_page_area_tests.rs"]
mod large_page_area_tests;
