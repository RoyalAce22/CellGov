//! Public vocabulary for comparison results.

use crate::observation::{ObservedEvent, ObservedOutcome};

/// Which fields to compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::VariantArray)]
pub enum CompareMode {
    /// Outcome + memory + full event sequence.
    Strict,
    /// Outcome + memory; events ignored.
    Memory,
    /// Outcome + events; memory ignored.
    Events,
    /// Outcome + events up to the shorter sequence length.
    Prefix,
}

/// Overall classification of a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::VariantArray)]
pub enum Classification {
    /// All compared fields agree.
    Match,
    /// One or more compared fields differ.
    Divergence,
    /// CellGov has no matching scenario for this test.
    Unsupported,
    /// Baselines disagree with each other; CellGov result is inconclusive.
    UnsettledOracle,
}

impl Classification {
    /// Whether this classification should produce a non-zero CLI
    /// exit code. `Match` and `Unsupported` exit 0; `Divergence` and
    /// `UnsettledOracle` exit 1.
    ///
    /// Exhaustive: every variant must declare its CI exit intent.
    pub fn exits_failure(&self) -> bool {
        match self {
            Classification::Divergence | Classification::UnsettledOracle => true,
            Classification::Match | Classification::Unsupported => false,
        }
    }
}

/// First byte-level difference between two named memory regions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryDivergence {
    /// Region name from the manifest.
    pub region: String,
    /// Byte offset of the first differing byte.
    pub offset: usize,
    /// Byte in the expected observation.
    pub expected: u8,
    /// Byte in the actual observation.
    pub actual: u8,
}

/// First difference between two event sequences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventDivergence {
    /// Index of the first differing event.
    pub index: usize,
    /// Event in the expected observation, if present.
    pub expected: Option<ObservedEvent>,
    /// Event in the actual observation, if present.
    pub actual: Option<ObservedEvent>,
}

/// Structured result of comparing two observations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareResult {
    /// Overall classification.
    pub classification: Classification,
    /// Mode used.
    pub mode: CompareMode,
    /// Set when outcomes differ.
    pub outcome_mismatch: Option<(ObservedOutcome, ObservedOutcome)>,
    /// First memory divergence, if any.
    pub memory_divergence: Option<MemoryDivergence>,
    /// First event divergence, if any.
    pub event_divergence: Option<EventDivergence>,
}

/// Result of comparing CellGov against multiple baselines.
///
/// Unsettled when baselines disagree: if any two baselines differ under `mode`,
/// classification is `UnsettledOracle` regardless of CellGov, and
/// `cellgov_result` is `None`. Otherwise CellGov is compared against the first
/// baseline (all baselines are equivalent under `mode` when the oracle settles).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiCompareResult {
    /// Overall classification.
    pub classification: Classification,
    /// Mode used for every sub-comparison.
    pub mode: CompareMode,
    /// First pairwise baseline disagreement, if oracles did not settle.
    pub oracle_divergence: Option<CompareResult>,
    /// CellGov vs. first baseline, if oracles settled.
    pub cellgov_result: Option<CompareResult>,
}
