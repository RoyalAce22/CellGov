//! Append-only record of every time a title's baseline moved.
//!
//! `dev record-anchors` appends one JSON object per line to one cell's
//! `boot_history.jsonl`. Nothing reads the file at runtime; it exists
//! so a reader can answer "when did this anchor change, and to what".
//! Lines carry no timestamp or commit id, so a re-record is
//! byte-reproducible: same code, same line. A run that changes nothing
//! appends nothing, so every line in the file is a real move.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::identity::RunIdentity;

/// The `changed` name a move of the identity triple is recorded under.
const IDENTITY_FIELD: &str = "identity";

/// One recorded move.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootHistoryEntry {
    /// Steps retired at the checkpoint.
    pub steps: u64,
    /// Terminal outcome, as `BootOutcome`'s debug spelling.
    pub outcome: String,
    /// Every witness value measured on this run.
    pub witnesses: BTreeMap<String, u64>,
    /// Fields recorded or moved on this line, sorted. Never empty --
    /// an unchanged run is not appended.
    pub changed: Vec<String>,
    /// Which firmware and title version this measurement was taken
    /// against. Empty on a line written before the store carried
    /// versions.
    #[serde(flatten)]
    pub identity: RunIdentity,
}

impl BootHistoryEntry {
    /// Build an entry, or `None` when nothing moved.
    ///
    /// `previous` is the last entry in the file, if any. With no
    /// `previous`, the result is always an entry. Its `changed` names:
    ///
    /// - the step count;
    /// - the outcome;
    /// - every witness;
    /// - a non-empty identity as `identity (first recorded)`.
    ///
    /// An identity triple that differs from the previous line's is a
    /// move on its own, even when every witness and the step count hold.
    pub fn new_if_changed(
        previous: Option<&Self>,
        steps: u64,
        outcome: &str,
        witnesses: BTreeMap<String, u64>,
        identity: RunIdentity,
    ) -> Option<Self> {
        let changed = match previous {
            None => {
                let mut names = vec!["steps".to_string(), "outcome".to_string()];
                names.extend(witnesses.keys().cloned());
                if let Some(name) = identity_move(&RunIdentity::default(), &identity) {
                    names.push(name);
                }
                names.sort();
                names
            }
            Some(prev) => {
                let mut names = Vec::new();
                if prev.steps != steps {
                    names.push("steps".to_string());
                }
                if prev.outcome != outcome {
                    names.push("outcome".to_string());
                }
                for (name, value) in &witnesses {
                    if prev.witnesses.get(name) != Some(value) {
                        names.push(name.clone());
                    }
                }
                // A witness the boot path stopped emitting is a move
                // in its own right.
                for name in prev.witnesses.keys() {
                    if !witnesses.contains_key(name) {
                        names.push(format!("{name} (no longer emitted)"));
                    }
                }
                if let Some(name) = identity_move(&prev.identity, &identity) {
                    names.push(name);
                }
                names.sort();
                names
            }
        };
        if changed.is_empty() {
            return None;
        }
        Some(Self {
            steps,
            outcome: outcome.to_string(),
            witnesses,
            changed,
            identity,
        })
    }
}

/// How this run's identity triple differs from the one the previous
/// line names, or `None` when it does not.
///
/// The caller appends only when something moved, so a first identity
/// triple over a pre-versioning line counts as a move. Without that
/// move, the last line keeps an empty identity and the axis never
/// moves again.
fn identity_move(previous: &RunIdentity, current: &RunIdentity) -> Option<String> {
    match (previous.is_empty(), current.is_empty()) {
        (true, true) => None,
        (true, false) => Some(format!("{IDENTITY_FIELD} (first recorded)")),
        (false, true) => Some(format!("{IDENTITY_FIELD} (no longer recorded)")),
        (false, false) if previous == current => None,
        (false, false) => Some(IDENTITY_FIELD.to_string()),
    }
}

/// A history line that failed to parse.
#[derive(Debug, thiserror::Error)]
#[error("line {line}: {source}")]
pub struct BootHistoryParseError {
    /// 1-based line number of the first unparseable line.
    pub line: usize,
    #[source]
    source: serde_json::Error,
}

/// Parse a history file, oldest first.
///
/// # Errors
///
/// Returns the 1-based line number and cause of the first
/// unparseable line.
pub fn parse(text: &str) -> Result<Vec<BootHistoryEntry>, BootHistoryParseError> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| {
            serde_json::from_str(l).map_err(|source| BootHistoryParseError {
                line: i + 1,
                source,
            })
        })
        .collect()
}

/// Render one entry as the single line to append.
///
/// # Errors
///
/// Returns the serialization failure.
pub fn render_line(entry: &BootHistoryEntry) -> Result<String, serde_json::Error> {
    Ok(serde_json::to_string(entry)? + "\n")
}

#[cfg(test)]
#[path = "tests/boot_history_identity_tests.rs"]
mod identity_tests;
