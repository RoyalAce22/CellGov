//! The oracle dispatch-gap overlay: the syscall ordinals the oracle's
//! dispatch table leaves unbound, and the operator-local file that
//! records them.
//!
//! The overlay stays out of the committed archive. The command computes
//! it from the operator's checkout of the oracle, and it names that
//! checkout's revision.

use std::collections::BTreeSet;

/// The marker a dispatch-table line carries when it binds a handler.
const BIND_MARKER: &str = "BIND_SYSC";

/// Key of the overlay's first line, which carries the checkout revision.
pub const OVERLAY_REVISION_KEY: &str = "revision";

/// The overlay's second line: the header of its one column.
pub const OVERLAY_ORDINAL_HEADER: &str = "ordinal";

/// The ordinals the oracle's dispatch-table source leaves unbound.
///
/// Each table line names its ordinal, or an inclusive `first-last`
/// range, at the start of a `//` comment. A line that names an ordinal
/// and carries no bind marker leaves it unbound. A line without a
/// leading ordinal in its comment is not a table row.
pub fn unbound_ordinals(dispatch_source: &str) -> BTreeSet<u64> {
    let mut ordinals = BTreeSet::new();
    for line in dispatch_source.lines() {
        let Some(comment) = line.split("//").nth(1) else {
            continue;
        };
        let digits: String = comment
            .chars()
            .take_while(|ch| ch.is_ascii_digit() || *ch == '-')
            .collect();
        let Some((first, last)) = digits.split_once('-').map_or_else(
            || digits.parse::<u64>().ok().map(|value| (value, value)),
            |(first, last)| Some((first.parse::<u64>().ok()?, last.parse::<u64>().ok()?)),
        ) else {
            continue;
        };
        if !line.contains(BIND_MARKER) {
            ordinals.extend(first..=last);
        }
    }
    ordinals
}

/// The overlay text for `ordinals` under the checkout `revision`.
pub fn overlay_text(revision: &str, ordinals: &BTreeSet<u64>) -> String {
    let mut text = format!("{OVERLAY_REVISION_KEY}\t{revision}\n{OVERLAY_ORDINAL_HEADER}\n");
    for ordinal in ordinals {
        text.push_str(&format!("{ordinal}\n"));
    }
    text
}

/// Why an overlay's text is not one [`overlay_text`] writes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OverlayParseError {
    /// Line 1 is not `revision<TAB><revision>`.
    #[error("line 1 is not `{OVERLAY_REVISION_KEY}<TAB><revision>`")]
    MissingRevision,
    /// Line 2 is not the column header.
    #[error("line 2 is not the `{OVERLAY_ORDINAL_HEADER}` column header")]
    MissingColumnHeader,
    /// A row below the header is not a syscall ordinal.
    #[error("line {line}: {row:?} is not a syscall ordinal")]
    MalformedRow {
        /// 1-based line number.
        line: usize,
        /// The row as written.
        row: String,
    },
}

/// The ordinals an overlay lists.
///
/// # Errors
///
/// A missing revision line or column header, or the first row that is
/// not a decimal `u64`.
pub fn parse_overlay(text: &str) -> Result<BTreeSet<u64>, OverlayParseError> {
    let mut lines = text.lines();
    if lines
        .next()
        .and_then(|line| line.split_once('\t'))
        .is_none_or(|(key, _)| key != OVERLAY_REVISION_KEY)
    {
        return Err(OverlayParseError::MissingRevision);
    }
    if lines.next() != Some(OVERLAY_ORDINAL_HEADER) {
        return Err(OverlayParseError::MissingColumnHeader);
    }
    lines
        .enumerate()
        .map(|(index, row)| {
            row.parse().map_err(|_| OverlayParseError::MalformedRow {
                line: index + 3,
                row: row.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/oracle_gap_tests.rs"]
mod tests;
