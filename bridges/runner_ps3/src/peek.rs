//! Read-only peeks of console memory through webMAN's `peek.lv1` and
//! `peek.lv2` pages, written as the TSV a peek capture commits.
//!
//! One request returns one [`PAGE`] of memory as an HTML hex dump. Each
//! dump row is the low 32 bits of its address, two spaces, and 16 bytes
//! as uppercase hex. The runner reads every page a plan names once per
//! read, in address order, and compares the reads of each word.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::error::RunnerPs3Error;
use crate::run::ConsoleOps;
use crate::transcript::Transcript;

/// The bytes one peek request returns.
pub const PAGE: u64 = 0x200;

/// The bytes of one dump row.
const ROW: u64 = 16;

/// The reads of each page when `--reads` is absent.
pub const DEFAULT_READS: u32 = 2;

/// The most reads one peek may make of each page.
pub const MAX_READS: u32 = 16;

/// The memory a peek reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Space {
    /// The hypervisor's memory, through `peek.lv1`.
    Lv1,
    /// The kernel's memory, through `peek.lv2`.
    Lv2,
}

impl Space {
    /// The space's word on the command line and in the stop list.
    pub fn name(self) -> &'static str {
        match self {
            Self::Lv1 => "lv1",
            Self::Lv2 => "lv2",
        }
    }

    /// The space a word names.
    pub fn from_name(word: &str) -> Option<Self> {
        [Self::Lv1, Self::Lv2]
            .into_iter()
            .find(|space| space.name() == word)
    }
}

impl fmt::Display for Space {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Why a peek page did not give its bytes.
#[derive(Debug, thiserror::Error)]
pub enum PeekError {
    /// webMAN answered `404`, so it serves no peek page.
    #[error("{path} answered 404, so this webMAN serves no peek page")]
    NotServed {
        /// The request path.
        path: String,
    },
    /// The page does not hold every dump row of the requested memory.
    #[error(
        "{path} holds {found} of the page's {} dump rows; the first missing row is 0x{missing:08x}",
        PAGE / ROW
    )]
    Rows {
        /// The request path.
        path: String,
        /// The rows found in address order before the first missing one.
        found: u64,
        /// The label of the first missing row.
        missing: u32,
    },
}

/// What one peek reads: words of one width in one space, each page
/// read `reads` times.
///
/// The fields are private, so [`PeekPlan::new`] is the only way to
/// make a plan, and [`read`] can index each page by every address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeekPlan {
    /// The memory.
    space: Space,
    /// The bytes of each word: 1, 2, 4 or 8.
    width: u8,
    /// Each word's address, in the order the TSV lists them.
    addresses: Vec<u64>,
    /// The reads of each page: 1 to [`MAX_READS`].
    reads: u32,
}

impl PeekPlan {
    /// The plan for these flags.
    ///
    /// # Errors
    ///
    /// [`RunnerPs3Error::Usage`] in each of these cases:
    ///
    /// - the width is not 1, 2, 4 or 8;
    /// - no address is given, or one is given twice;
    /// - an address is not a multiple of the width;
    /// - `reads` is 0 or above [`MAX_READS`].
    pub fn new(
        space: Space,
        width: u8,
        addresses: Vec<u64>,
        reads: u32,
    ) -> Result<Self, RunnerPs3Error> {
        let usage = |message: String| Err(RunnerPs3Error::Usage(message));
        if !matches!(width, 1 | 2 | 4 | 8) {
            return usage(format!("--width takes 1, 2, 4 or 8, not {width}"));
        }
        if addresses.is_empty() {
            return usage("--address names no address".to_string());
        }
        if !(1..=MAX_READS).contains(&reads) {
            return usage(format!("--reads takes 1 to {MAX_READS}, not {reads}"));
        }
        let mut seen = BTreeSet::new();
        for &address in &addresses {
            if address % u64::from(width) != 0 {
                return usage(format!(
                    "address 0x{address:016x} is not a multiple of the width {width}"
                ));
            }
            if !seen.insert(address) {
                return usage(format!("address 0x{address:016x} is given twice"));
            }
        }
        Ok(Self {
            space,
            width,
            addresses,
            reads,
        })
    }

    /// The first address of every page the plan reads, ascending. An
    /// address that is a multiple of the width never crosses a page.
    pub fn pages(&self) -> BTreeSet<u64> {
        self.addresses.iter().map(|a| a & !(PAGE - 1)).collect()
    }

    /// The memory.
    pub fn space(&self) -> Space {
        self.space
    }

    /// The bytes of each word: 1, 2, 4 or 8.
    pub fn width(&self) -> u8 {
        self.width
    }

    /// Each word's address, in the order the TSV lists them.
    pub fn addresses(&self) -> &[u64] {
        &self.addresses
    }

    /// The reads of each page.
    pub fn reads(&self) -> u32 {
        self.reads
    }
}

/// One word a peek read, with its value at each read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeekRow {
    /// The word's address.
    pub address: u64,
    /// The word at each read, in read order.
    pub values: Vec<u64>,
}

/// The request path for the page at `page` in `space`.
pub fn request_path(space: Space, page: u64) -> String {
    format!("/peek.{space}?{page:016X}")
}

/// The [`PAGE`] bytes the dump rows of `html` give for the page at
/// `page`. The rows are found in address order after the page's `<pre>`.
///
/// # Errors
///
/// [`PeekError::Rows`] naming the first row of the page the HTML does
/// not hold.
pub fn parse_page(html: &str, page: u64, path: &str) -> Result<Vec<u8>, PeekError> {
    let mut rest = html.find("<pre>").map_or("", |at| &html[at..]);
    let mut bytes = Vec::with_capacity(PAGE as usize);
    for row in 0..PAGE / ROW {
        let label = (page + row * ROW) as u32;
        let missing = || PeekError::Rows {
            path: path.to_string(),
            found: row,
            missing: label,
        };
        let head = format!("{label:08X}  ");
        let at = rest.find(&head).ok_or_else(missing)?;
        rest = &rest[at + head.len()..];
        for _ in 0..ROW {
            let byte = rest
                .get(..2)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                .ok_or_else(missing)?;
            bytes.push(byte);
            rest = rest.get(2..).ok_or_else(missing)?;
            rest = rest.strip_prefix(' ').unwrap_or(rest);
        }
    }
    Ok(bytes)
}

/// Read every page of `plan` from `console`, `plan.reads` times, and
/// return each word's values.
///
/// # Errors
///
/// A transport failure, [`PeekError::NotServed`] for a page webMAN
/// answers with `404`, and [`PeekError::Rows`] for a page whose dump
/// does not parse.
pub fn read<C: ConsoleOps>(
    console: &mut C,
    plan: &PeekPlan,
    transcript: &mut Transcript,
) -> Result<Vec<PeekRow>, RunnerPs3Error> {
    let pages = plan.pages();
    let mut reads: BTreeMap<u64, Vec<Vec<u8>>> = BTreeMap::new();
    for _ in 0..plan.reads {
        for &page in &pages {
            let path = request_path(plan.space, page);
            let body = console
                .fetch(&path, transcript)?
                .ok_or_else(|| PeekError::NotServed { path: path.clone() })?;
            let bytes = parse_page(&String::from_utf8_lossy(&body), page, &path)?;
            reads.entry(page).or_default().push(bytes);
        }
    }
    // PeekPlan::new is the only constructor: reads >= 1 puts every page
    // in `reads`, and an address that is a multiple of a width that
    // divides PAGE keeps its word inside the PAGE bytes parse_page gives.
    let width = usize::from(plan.width);
    Ok(plan
        .addresses
        .iter()
        .map(|&address| {
            let page = address & !(PAGE - 1);
            let offset = (address - page) as usize;
            let values = reads[&page]
                .iter()
                .map(|bytes| {
                    bytes[offset..offset + width]
                        .iter()
                        .fold(0u64, |word, &byte| (word << 8) | u64::from(byte))
                })
                .collect();
            PeekRow { address, values }
        })
        .collect())
}

/// The value column's name for words of `width` bytes.
fn column(width: u8) -> &'static str {
    match width {
        1 => "byte",
        2 => "halfword",
        4 => "word",
        _ => "doubleword",
    }
}

/// The TSV a peek capture commits: a header, then one line per word
/// with its address, its read count, whether every read agrees, and its
/// value at the first read.
pub fn render_tsv(width: u8, rows: &[PeekRow]) -> String {
    let digits = 2 * usize::from(width);
    let mut tsv = format!("address\treads\tagree\t{}\n", column(width));
    for row in rows {
        let first = row.values.first().copied().unwrap_or_default();
        let agree = if row.values.iter().all(|&value| value == first) {
            "yes"
        } else {
            "no"
        };
        tsv.push_str(&format!(
            "0x{:016x}\t{}\t{agree}\t0x{first:0digits$x}\n",
            row.address,
            row.values.len()
        ));
    }
    tsv
}

#[cfg(test)]
#[path = "tests/peek_tests.rs"]
mod tests;
