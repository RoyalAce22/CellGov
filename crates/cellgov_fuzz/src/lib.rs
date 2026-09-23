//! Deterministic fuzz engines for the PPU and SPU interpreters. Callers provide host policy, and each case derives from its configuration and iteration number.

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod ppu;
pub mod report;
pub mod spu;
pub mod sweep;

mod boundary;
mod error;
mod rng;

const MAX_SEQUENCE_WORDS: usize = 65_536;
const MAX_RETAINED_FINDINGS: usize = 1_024;

pub use boundary::TargetPanicPayload;
pub use error::{
    ConfigurationError, FuzzError, GeneratorError, InvariantError, ReductionError,
    ReferenceDisagreement, ReplayVersionError, ReportingError, SynchronizationError, WorkerError,
};
pub use report::{
    CheckIdentity, DivergenceClass, Finding, FindingKind, FuzzReport, FuzzRun, FuzzTarget,
    InstructionIdentity, OutcomeIdentity, ReductionOutcome, ReplayCoordinates, RunOutcome,
    SemanticFingerprint, CAMPAIGN_VERSION,
};
pub use sweep::{ppu_decode_partition, spu_decode_partition, DecodePanic, DecodeSweepReport};

/// Reusable configuration shared by all fuzz engines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FuzzConfig {
    /// Master seed.
    pub seed: u64,
    /// First iteration number.
    pub first_iteration: u64,
    /// Number of cases to run.
    pub iterations: u64,
    /// Maximum number of detailed findings retained in memory.
    pub max_findings: usize,
    /// Instruction count used by sequence engines.
    pub sequence_words: usize,
}

impl Default for FuzzConfig {
    fn default() -> Self {
        Self {
            seed: 1,
            first_iteration: 0,
            iterations: 1_000_000,
            max_findings: 20,
            sequence_words: 32,
        }
    }
}

impl FuzzConfig {
    /// Iteration numbers for this run.
    pub fn iterations(self) -> impl Iterator<Item = u64> {
        (0..self.iterations).map(move |offset| self.first_iteration.wrapping_add(offset))
    }

    pub(crate) fn validate(self, sequence_limit: Option<usize>) -> Result<(), ConfigurationError> {
        if self.iterations == 0 {
            return Err(ConfigurationError::ZeroIterations);
        }
        if let Some(maximum) = sequence_limit {
            if self.sequence_words == 0 {
                return Err(ConfigurationError::ZeroSequenceWords);
            }
            if self.sequence_words > maximum {
                return Err(ConfigurationError::SequenceTooLong {
                    requested: self.sequence_words,
                    maximum,
                });
            }
        }
        if self.max_findings > MAX_RETAINED_FINDINGS {
            return Err(ConfigurationError::TooManyRetainedFindings {
                requested: self.max_findings,
                maximum: MAX_RETAINED_FINDINGS,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/lib_tests.rs"]
mod tests;
