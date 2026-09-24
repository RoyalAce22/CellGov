//! Builds the operator-local oracle dispatch-gap overlay.

use std::path::Path;

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::paths::workspace_root;

/// Key of the overlay's first line, which carries the checkout revision.
const REVISION_KEY: &str = "revision";

/// The overlay's second line: the header of its one column.
const ORDINAL_HEADER: &str = "ordinal";

/// Why an overlay's text is not one this command writes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum OverlayParseError {
    /// Line 1 is not `revision<TAB><revision>`.
    #[error("line 1 is not `{REVISION_KEY}<TAB><revision>`")]
    MissingRevision,
    /// Line 2 is not the column header.
    #[error("line 2 is not the `{ORDINAL_HEADER}` column header")]
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

/// The ordinals an overlay this command wrote lists.
///
/// # Errors
///
/// A missing revision line or column header, or the first row that is
/// not a decimal `u64`.
pub(crate) fn parse_overlay(
    text: &str,
) -> Result<std::collections::BTreeSet<u64>, OverlayParseError> {
    let mut lines = text.lines();
    if lines
        .next()
        .and_then(|line| line.split_once('\t'))
        .is_none_or(|(key, _)| key != REVISION_KEY)
    {
        return Err(OverlayParseError::MissingRevision);
    }
    if lines.next() != Some(ORDINAL_HEADER) {
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

/// Writes the source revision and each unbound table slot.
///
/// # Errors
///
/// Returns an error if:
///
/// - Git cannot read the checkout revision.
/// - The command cannot read the dispatch table.
/// - The command cannot write the overlay.
pub(crate) fn run(vfs_flag: Option<&Path>) -> Result<CommandExitCode, CommandError> {
    let root = vfs_flag.unwrap_or_else(|| Path::new("vfs"));
    let checkout = ["rpc", "s3-src"].concat();
    let checkout_root = workspace_root().join("tools").join(checkout);
    let source = checkout_root.join(["rpc", "s3/Emu/Cell/lv2/lv2.cpp"].concat());
    if !source.exists() {
        println!("oracle gap: not computed -- local oracle checkout is unavailable");
        return Ok(CommandExitCode::SUCCESS);
    }
    let revision = std::process::Command::new("git")
        .arg("-C")
        .arg(&checkout_root)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| {
            CommandError::failed(format!("oracle gap: read checkout revision: {error}"))
        })?;
    if !revision.status.success() {
        return Err(CommandError::failed(
            "oracle gap: checkout has no readable revision",
        ));
    }
    let table = std::fs::read_to_string(&source).map_err(|error| {
        CommandError::failed(format!("oracle gap: read dispatch table: {error}"))
    })?;
    let mut ordinals = std::collections::BTreeSet::new();
    for line in table.lines() {
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
        if !line.contains("BIND_SYSC") {
            ordinals.extend(first..=last);
        }
    }
    let out = root.join(".cellgov/oracle-gap.tsv");
    std::fs::create_dir_all(root.join(".cellgov"))
        .map_err(|error| CommandError::failed(format!("oracle gap: create overlay: {error}")))?;
    let mut text = format!(
        "{REVISION_KEY}\t{}\n{ORDINAL_HEADER}\n",
        String::from_utf8_lossy(&revision.stdout).trim()
    );
    for ordinal in ordinals {
        text.push_str(&format!("{ordinal}\n"));
    }
    std::fs::write(&out, text)
        .map_err(|error| CommandError::failed(format!("oracle gap: write overlay: {error}")))?;
    println!("oracle gap: wrote {}", out.display());
    Ok(CommandExitCode::SUCCESS)
}

#[cfg(test)]
#[path = "tests/oracle_gap_tests.rs"]
mod tests;
