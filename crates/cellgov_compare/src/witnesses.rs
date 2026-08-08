//! Per-title witness baselines: the measured counters a boot must
//! reproduce, and how strictly each is held.
//!
//! Recorded into `BootSummary` by the boot path and asserted against
//! by the title-witness test suite. Both sides read this one file, so
//! a recorded value and an asserted value cannot disagree.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Sentinel the boot path prints once the title's boot inputs
/// (manifest + decrypted EBOOT image) resolve.
///
/// Beyond this line, a failing run is a boot failure, never a missing
/// dump: the suites treat its absence together with
/// [`TITLE_NOT_INSTALLED_SENTINEL`] to split "skip this title" from
/// "the boot ran and broke".
pub const BOOT_STARTED_SENTINEL: &str = "BENCH_BOOT_INPUTS_RESOLVED:";

/// Sentinel the boot path prints when a title's dump is absent from
/// this machine: the EBOOT candidate walk found no file at all, or
/// the title's directory does not exist.
///
/// Emitted only for missing-dump causes; a dump that exists but fails
/// to decrypt or parse dies without this marker.
pub const TITLE_NOT_INSTALLED_SENTINEL: &str = "BENCH_TITLE_NOT_INSTALLED:";

/// How strictly a recorded witness is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WitnessClass {
    /// `observed == expected`. For values whose movement is always a
    /// finding: invariant-break counts, anchors.
    Exact,
    /// `observed >= expected`. For counters where more work is fine
    /// but losing coverage is a regression.
    AtLeast,
    /// `observed == 0`. The boot does not reach this path.
    Absent,
    /// Never fails. Recorded so the value shows up in the diff.
    Informational,
}

impl WitnessClass {
    /// Class assigned when a witness is first recorded.
    ///
    /// Counters that fired become floors, silent ones become
    /// `Absent`. Anything needing `Exact` is named by
    /// [`Self::is_exact_by_default`]; other classes can be promoted
    /// by hand-editing the committed baseline.
    pub fn classify_on_record(name: &str, observed: u64) -> Self {
        if Self::is_exact_by_default(name) {
            Self::Exact
        } else if observed == 0 {
            Self::Absent
        } else {
            Self::AtLeast
        }
    }

    /// Witnesses whose every movement is a finding.
    pub fn is_exact_by_default(name: &str) -> bool {
        matches!(name, "host_invariant_breaks" | "vrsave_written")
    }

    /// Check one observation, returning why it failed.
    ///
    /// # Errors
    ///
    /// Returns [`WitnessMismatch`] when the observation violates the
    /// class.
    pub fn check(self, name: &str, expected: u64, observed: u64) -> Result<(), WitnessMismatch> {
        let ok = match self {
            Self::Exact => observed == expected,
            Self::AtLeast => observed >= expected,
            Self::Absent => observed == 0,
            Self::Informational => true,
        };
        if ok {
            return Ok(());
        }
        Err(WitnessMismatch {
            name: name.to_string(),
            class: self,
            expected,
            observed,
        })
    }
}

/// One recorded witness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Witness {
    /// Value measured when the baseline was recorded. Booleans are
    /// recorded as 0 or 1.
    pub value: u64,
    /// How strictly [`value`](Self::value) is held.
    pub class: WitnessClass,
}

/// A title's full witness baseline, keyed by witness name.
///
/// Names match the `BENCH_*` stderr keys the boot path emits, so a
/// new witness needs no schema change here -- record it and it
/// appears.
pub type WitnessSet = BTreeMap<String, Witness>;

/// A single witness that failed its class check.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("witness {name}: expected {class:?} {expected}, observed {observed}")]
pub struct WitnessMismatch {
    /// Witness name, e.g. `"ldarx"`.
    pub name: String,
    /// Class that rejected the observation.
    pub class: WitnessClass,
    /// Recorded baseline value.
    pub expected: u64,
    /// Value this run produced.
    pub observed: u64,
}

/// One way a baseline check can fail.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WitnessFailure {
    /// The observed value violated the recorded class.
    #[error(transparent)]
    Mismatch(WitnessMismatch),
    /// The recorded witness's emitting `BENCH_*` line never appeared
    /// in the boot output; observed 0 and never-emitted are
    /// indistinguishable at the value level.
    #[error("witness {name}: its line {line} never appeared in the boot output; the emitter is gone or the boot died before it")]
    LineAbsent {
        /// Witness name.
        name: String,
        /// The `BENCH_*` prefix that should have carried it.
        line: &'static str,
    },
    /// The baseline records a witness this build's line table does not
    /// know.
    #[error("witness {name}: recorded in the baseline but unknown to this build; re-record or migrate the baseline")]
    UnknownWitness {
        /// Witness name.
        name: String,
    },
}

/// Compare observations against a baseline.
///
/// Returns every failure rather than the first, so one run reports
/// the whole delta. A witness present in `observed` but absent from
/// `baseline` is reported via [`unrecorded`]. The reverse splits by
/// cause: if the witness's emitting line appeared, the value check
/// runs with `observed` 0; if the line never appeared, that is
/// [`WitnessFailure::LineAbsent`] regardless of class.
pub fn check_all(
    baseline: &WitnessSet,
    observed: &crate::witness_parse::ParsedWitnesses,
) -> Vec<WitnessFailure> {
    baseline
        .iter()
        .filter_map(|(name, w)| {
            let Some(line) = crate::witness_parse::line_of(name) else {
                return Some(WitnessFailure::UnknownWitness { name: name.clone() });
            };
            if !observed.seen_lines.contains(line) {
                return Some(WitnessFailure::LineAbsent {
                    name: name.clone(),
                    line,
                });
            }
            let seen = observed.values.get(name).copied().unwrap_or(0);
            w.class
                .check(name, w.value, seen)
                .err()
                .map(WitnessFailure::Mismatch)
        })
        .collect()
}

/// Witness names this run produced that the baseline does not carry.
///
/// A new witness is not a failure -- it means the boot path grew one
/// and the baseline needs re-recording.
pub fn unrecorded(baseline: &WitnessSet, observed: &BTreeMap<String, u64>) -> Vec<String> {
    observed
        .keys()
        .filter(|name| !baseline.contains_key(*name))
        .cloned()
        .collect()
}

/// Build a baseline from a run, preserving the class of any witness
/// the previous baseline already carried.
///
/// Exception: an `Absent` witness measured nonzero reclassifies as if
/// first recorded, since `Absent` means "observed == 0" and keeping
/// it would write a baseline that fails its own check on every run.
pub fn record(previous: Option<&WitnessSet>, observed: &BTreeMap<String, u64>) -> WitnessSet {
    observed
        .iter()
        .map(|(name, &value)| {
            let class = match previous.and_then(|p| p.get(name)) {
                Some(w) if w.class == WitnessClass::Absent && value != 0 => {
                    WitnessClass::classify_on_record(name, value)
                }
                Some(w) => w.class,
                None => WitnessClass::classify_on_record(name, value),
            };
            (name.clone(), Witness { value, class })
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/witnesses_tests.rs"]
mod tests;
