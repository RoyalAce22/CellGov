//! Field-level value types referenced from the section structs.

use serde::Deserialize;

use crate::observation::ObservedOutcome;
#[cfg(feature = "rpcs3-runner")]
use crate::runner_rpcs3::Rpcs3Decoder;

/// A memory region to observe, as declared in the manifest.
#[derive(Debug, Clone, Deserialize)]
pub struct MemoryRegionSpec {
    /// Region name (used in reports and baseline keys).
    pub name: String,
    /// Guest address of the region start.
    pub addr: u64,
    /// Size in bytes.
    pub size: u64,
}

/// RPCS3 decoder selection.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DecoderField {
    /// PPU + SPU interpreter.
    #[default]
    Interpreter,
    /// PPU + SPU LLVM recompiler.
    Llvm,
}

#[cfg(feature = "rpcs3-runner")]
impl From<DecoderField> for Rpcs3Decoder {
    fn from(d: DecoderField) -> Self {
        match d {
            DecoderField::Interpreter => Rpcs3Decoder::Interpreter,
            DecoderField::Llvm => Rpcs3Decoder::Llvm,
        }
    }
}

/// Expected-outcome field (lowercase string in TOML).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutcomeField {
    /// Test ran to completion.
    Completed,
    /// Test stalled (deadlock or livelock).
    Stalled,
    /// Test exceeded its time or step budget.
    Timeout,
    /// Test faulted.
    Fault,
}

impl From<OutcomeField> for ObservedOutcome {
    fn from(o: OutcomeField) -> Self {
        match o {
            OutcomeField::Completed => ObservedOutcome::Completed,
            OutcomeField::Stalled => ObservedOutcome::Stalled,
            OutcomeField::Timeout => ObservedOutcome::Timeout,
            OutcomeField::Fault => ObservedOutcome::Fault,
        }
    }
}

pub(super) fn default_max_steps() -> usize {
    10000
}

pub(super) fn default_timeout_ms() -> u64 {
    5000
}
