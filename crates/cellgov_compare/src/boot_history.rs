//! Append-only record of every time a title's baseline moved.
//!
//! One JSON object per line in
//! `tests/fixtures/<content-id>/cellgov/boot_history.jsonl`, appended
//! by `record-anchors` and read by nobody at runtime -- it exists so a
//! reader can answer "when did this anchor change, and to what".
//! Lines carry no timestamp or commit id, so a re-record is
//! byte-reproducible: same code, same line. A run that changes nothing
//! appends nothing, so every line in the file is a real move.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One recorded move.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootHistoryEntry {
    /// Steps retired at the checkpoint.
    pub steps: u64,
    /// Terminal outcome, as `BootOutcome`'s debug spelling.
    pub outcome: String,
    /// Every witness value measured on this run.
    pub witnesses: BTreeMap<String, u64>,
    /// Fields that differ from the previous entry, sorted. Never
    /// empty -- an unchanged run is not appended.
    pub changed: Vec<String>,
}

impl BootHistoryEntry {
    /// Build an entry, or `None` when nothing moved.
    ///
    /// `previous` is the last entry in the file, if any. A first
    /// recording always produces an entry, with `changed` naming
    /// every field as newly recorded.
    pub fn new_if_changed(
        previous: Option<&Self>,
        steps: u64,
        outcome: &str,
        witnesses: BTreeMap<String, u64>,
    ) -> Option<Self> {
        let changed = match previous {
            None => {
                let mut names = vec!["steps".to_string(), "outcome".to_string()];
                names.extend(witnesses.keys().cloned());
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
        })
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
