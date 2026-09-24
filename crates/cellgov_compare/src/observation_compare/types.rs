//! The comparison result types and the verdict predicates over them.

use serde::{Deserialize, Serialize};

use crate::identity::{identity_report, RunIdentity};
use crate::observation::{ObservedEvent, ObservedHashes, ObservedOutcome};

/// Aggregate verdict from comparing two [`Observation`](crate::observation::Observation)s.
///
/// One field per compared dimension; `a_runner` / `b_runner` carry
/// the runner names from each observation's metadata so renderers can
/// label divergence lines without re-threading the source observations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationCompareResult {
    /// True iff the two observations reported the same [`ObservedOutcome`].
    pub outcome_match: bool,
    /// Outcome reported by observation `a` (preserved verbatim for
    /// renderer use even when `outcome_match` is true).
    pub a_outcome: ObservedOutcome,
    /// Outcome reported by observation `b`.
    pub b_outcome: ObservedOutcome,
    /// Per-region comparison summary; see [`RegionCompareSummary`].
    pub region_compare: RegionCompareSummary,
    /// Event-sequence comparison verdict.
    pub event_compare: EventCompare,
    /// State-hash comparison verdict (same-runner only counts as
    /// divergence; see [`StateHashCompare`]).
    pub state_hash_compare: StateHashCompare,
    /// Step-count comparison verdict (same-runner only counts as
    /// divergence; see [`StepCompare`]).
    pub step_compare: StepCompare,
    /// Runner name from `a.metadata.runner` (e.g., `"cellgov"`,
    /// `"rpcs3"`).
    pub a_runner: String,
    /// Runner name from `b.metadata.runner`.
    pub b_runner: String,
    /// Identity triple from `a.identity`.
    #[serde(default, skip_serializing_if = "RunIdentity::is_empty")]
    pub a_identity: RunIdentity,
    /// Identity triple from `b.identity`.
    #[serde(default, skip_serializing_if = "RunIdentity::is_empty")]
    pub b_identity: RunIdentity,
}

/// Aggregate of per-region pair outcomes plus the raw region counts.
///
/// `a_count` / `b_count` are the lengths of each side's
/// `memory_regions` vector. When the counts disagree, `pairs` is
/// empty -- no per-pair walk happens because there is no
/// well-defined zipping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionCompareSummary {
    /// Number of regions in observation `a`.
    pub a_count: usize,
    /// Number of regions in observation `b`.
    pub b_count: usize,
    /// Per-pair outcomes in observation order. Empty when region
    /// counts disagree.
    pub pairs: Vec<RegionPairOutcome>,
}

/// Outcome for one zipped pair of [`NamedMemoryRegion`](crate::observation::NamedMemoryRegion)s.
///
/// Variants are checked in order: identity, then length, then byte
/// content. The first mismatch terminates the pair (a length
/// mismatch suppresses the byte walk), but subsequent region pairs
/// are still walked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RegionPairOutcome {
    /// Regions share name, address, length, and byte content.
    Match {
        /// Region name shared by both sides.
        name: String,
        /// Guest base address shared by both sides.
        addr: u64,
        /// Region length in bytes.
        length: u64,
    },
    /// Region pair disagrees on name or guest base address.
    ///
    /// Distinguishing name vs address is left to the renderer; both
    /// pairs are recorded so callers can report the actual mismatch.
    IdentityMismatch {
        /// Region name from observation `a`.
        a_name: String,
        /// Guest base address from observation `a`.
        a_addr: u64,
        /// Region name from observation `b`.
        b_name: String,
        /// Guest base address from observation `b`.
        b_addr: u64,
    },
    /// Region pair shares identity but disagrees on byte length.
    /// Suppresses the byte-level walk for this pair.
    LengthMismatch {
        /// Region name (matches on both sides).
        name: String,
        /// Byte length of observation `a`'s data buffer.
        a_length: u64,
        /// Byte length of observation `b`'s data buffer.
        b_length: u64,
    },
    /// Region pair shares identity and length but has at least one
    /// differing byte.
    ByteDivergence {
        /// Region name (matches on both sides).
        name: String,
        /// Guest base address (matches on both sides).
        addr: u64,
        /// Region length in bytes (matches on both sides).
        length: u64,
        /// Coalesced runs of differing bytes within the region, in
        /// ascending offset order. A single differing byte produces
        /// one entry with `length == 1`; a contiguous run produces
        /// one entry with `length == N`.
        bytes: Vec<ByteDivergence>,
    },
}

/// One coalesced run of differing bytes within a region.
///
/// Only the first byte pair (`a_byte`, `b_byte`) is recorded; the
/// full run requires holding the source observations alongside this
/// result. Consumers that need every diverging byte must reopen the
/// source observations and re-slice by `(name, offset, length)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteDivergence {
    /// Byte offset within the region where this run starts.
    pub offset: u64,
    /// Always >= 1; the producer asserts this and classifier
    /// consumers may `debug_assert!` it too.
    pub length: u64,
    /// Byte from observation `a` at `offset` (only the first pair in
    /// the run is recorded; consumers needing more must reopen the
    /// source observation).
    pub a_byte: u8,
    /// Byte from observation `b` at `offset`.
    pub b_byte: u8,
}

/// Step-count comparison verdict.
///
/// Step counts are reported only by runners that expose an internal
/// step counter (CellGov). Same-runner mismatches indicate
/// non-determinism; cross-runner mismatches are notes because the
/// two runners can legitimately reach the same observable state via
/// different amounts of internal work.
///
/// [Martignoni2009 p:126 s:2] A deviation confined to internal state
/// is invisible to the emulated program and is not a defect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepCompare {
    /// Neither observation carries a step count.
    NoStepInfo,
    /// Both observations report the same step count.
    Equal {
        /// Step count shared by both sides.
        steps: usize,
    },
    /// Differing step counts within the same runner: a determinism
    /// failure.
    SameRunnerMismatch {
        /// Step count from observation `a`.
        a: usize,
        /// Step count from observation `b`.
        b: usize,
    },
    /// Differing step counts across runners: informational only.
    CrossRunnerNote {
        /// Step count from observation `a`.
        a: usize,
        /// Step count from observation `b`.
        b: usize,
    },
    /// One observation reports a step count, the other does not.
    /// Producer guarantees exactly one of `a` / `b` is `Some`.
    OneMissing {
        /// Step count from observation `a`, or `None` if absent.
        a: Option<usize>,
        /// Step count from observation `b`, or `None` if absent.
        b: Option<usize>,
    },
}

/// Event-sequence comparison verdict.
///
/// Equality is strict by index: the producer is responsible for
/// emitting events in the normalized order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventCompare {
    /// Both sequences have identical events at every index.
    Equal {
        /// Number of events in the (matching) sequences.
        count: usize,
    },
    /// Sequences differ in length; per-index walk is suppressed.
    LengthMismatch {
        /// Number of events in observation `a`.
        a: usize,
        /// Number of events in observation `b`.
        b: usize,
    },
    /// First index where the two sequences disagree. `index < min(a_len, b_len)`.
    FirstIndexDiffers {
        /// Zero-based index of the first differing event.
        index: usize,
        /// Event from observation `a` at `index`.
        a: ObservedEvent,
        /// Event from observation `b` at `index`.
        b: ObservedEvent,
    },
}

/// CellGov state-hash comparison verdict.
///
/// The RPCS3 adapter sets `state_hashes` to `None` (see
/// [`ObservedHashes`] doc), so cross-runner pairs land in
/// `OneMissing` or `NoHashInfo` and are never a divergence. A
/// same-runner pair carrying differing hashes is a determinism
/// failure analogous to [`StepCompare::SameRunnerMismatch`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StateHashCompare {
    /// Neither observation carries state hashes.
    NoHashInfo,
    /// Both observations carry matching state hashes.
    Equal,
    /// One side carries state hashes and the other does not.
    OneMissing {
        /// True iff observation `a` carried state hashes.
        a_present: bool,
        /// True iff observation `b` carried state hashes.
        b_present: bool,
    },
    /// Same-runner pair with differing hashes; a determinism failure.
    SameRunnerMismatch {
        /// Hashes from observation `a`.
        a: ObservedHashes,
        /// Hashes from observation `b`.
        b: ObservedHashes,
    },
    /// Cross-runner pair with differing hashes; informational only
    /// (state-hash shape is CellGov-defined).
    CrossRunnerNote {
        /// Hashes from observation `a`.
        a: ObservedHashes,
        /// Hashes from observation `b`.
        b: ObservedHashes,
    },
}

impl RegionCompareSummary {
    /// True iff the two observations reported different numbers of
    /// regions. When true, `pairs` is empty.
    pub fn is_count_mismatch(&self) -> bool {
        self.a_count != self.b_count
    }

    /// True iff at least one zipped pair is anything other than
    /// [`RegionPairOutcome::Match`].
    pub fn has_pair_divergence(&self) -> bool {
        self.pairs.iter().any(|p| !p.is_match())
    }

    /// Total bytes across all [`RegionPairOutcome::Match`] entries
    /// (sum of their `length` fields). Used by the MATCH summary line.
    pub fn matched_bytes(&self) -> u64 {
        self.pairs
            .iter()
            .filter_map(|p| match p {
                RegionPairOutcome::Match { length, .. } => Some(*length),
                _ => None,
            })
            .sum()
    }

    /// Number of [`RegionPairOutcome::Match`] entries in `pairs`.
    pub fn matched_regions(&self) -> u64 {
        self.pairs.iter().filter(|p| p.is_match()).count() as u64
    }
}

impl RegionPairOutcome {
    /// Whether this pair is a [`RegionPairOutcome::Match`].
    pub fn is_match(&self) -> bool {
        match self {
            RegionPairOutcome::Match { .. } => true,
            RegionPairOutcome::IdentityMismatch { .. }
            | RegionPairOutcome::LengthMismatch { .. }
            | RegionPairOutcome::ByteDivergence { .. } => false,
        }
    }

    /// Inverse of [`Self::is_match`].
    pub fn is_divergent(&self) -> bool {
        !self.is_match()
    }
}

impl EventCompare {
    /// Whether this verdict represents two equal event sequences.
    pub fn is_equal(&self) -> bool {
        match self {
            EventCompare::Equal { .. } => true,
            EventCompare::LengthMismatch { .. } | EventCompare::FirstIndexDiffers { .. } => false,
        }
    }

    /// True iff the event sequences are non-equal.
    pub fn is_divergent(&self) -> bool {
        !self.is_equal()
    }
}

impl StateHashCompare {
    /// Whether this verdict drives a non-zero exit code (only
    /// same-runner hash mismatches qualify; cross-runner mismatches
    /// are informational).
    pub fn is_same_runner_mismatch(&self) -> bool {
        match self {
            StateHashCompare::SameRunnerMismatch { .. } => true,
            StateHashCompare::NoHashInfo
            | StateHashCompare::Equal
            | StateHashCompare::OneMissing { .. }
            | StateHashCompare::CrossRunnerNote { .. } => false,
        }
    }

    /// True iff this verdict drives a non-zero exit code.
    pub fn is_divergent(&self) -> bool {
        self.is_same_runner_mismatch()
    }
}

impl StepCompare {
    /// Whether this verdict represents a same-runner step-count
    /// mismatch (a determinism failure). Cross-runner mismatches
    /// are informational.
    pub fn is_same_runner_mismatch(&self) -> bool {
        match self {
            StepCompare::SameRunnerMismatch { .. } => true,
            StepCompare::NoStepInfo
            | StepCompare::Equal { .. }
            | StepCompare::CrossRunnerNote { .. }
            | StepCompare::OneMissing { .. } => false,
        }
    }
}

impl ObservationCompareResult {
    /// True iff anything in this result drives a non-zero exit code:
    /// outcome mismatch, any region-side mismatch, an event-sequence
    /// mismatch, a same-runner step mismatch, or a same-runner
    /// state-hash mismatch. Cross-runner step / state-hash mismatches
    /// are notes, not divergences. An identity-triple mismatch is also
    /// a note; see [`Self::identity_report`].
    pub fn has_divergence(&self) -> bool {
        !self.outcome_match
            || self.region_compare.is_count_mismatch()
            || self.region_compare.has_pair_divergence()
            || self.event_compare.is_divergent()
            || self.state_hash_compare.is_divergent()
            || self.step_compare.is_same_runner_mismatch()
    }

    /// True iff outcomes and regions match and both observations
    /// reported zero regions. Drives the CLI's WARN line on stderr
    /// for the "nothing was compared" case.
    pub fn is_vacuous(&self) -> bool {
        self.outcome_match
            && !self.region_compare.is_count_mismatch()
            && !self.region_compare.has_pair_divergence()
            && self.region_compare.a_count == 0
            && self.region_compare.b_count == 0
    }

    /// Returns `Some((a_steps, b_steps))` exactly when the CLI
    /// prints its `NOTE: step counts differ ...` stderr line; cross-runner
    /// step divergence is informational, not a divergence.
    pub fn cross_runner_step_note(&self) -> Option<(usize, usize)> {
        if let StepCompare::CrossRunnerNote { a, b } = self.step_compare {
            Some((a, b))
        } else {
            None
        }
    }

    /// Both sides' identity triples, then the cross-triple warning when
    /// they disagree.
    ///
    /// Pass labels that tell the two sides apart, such as the two file
    /// paths: the runner names are often the same string.
    pub fn identity_report(&self, a_label: &str, b_label: &str) -> Vec<String> {
        identity_report(&self.a_identity, a_label, &self.b_identity, b_label)
    }
}
