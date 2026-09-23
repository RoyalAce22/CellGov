//! Deterministic fuzz engines for the PPU and SPU interpreters; callers provide host policy and campaign configuration.

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod ppu;
pub mod report;
pub mod spu;
pub mod sweep;

mod boundary;
mod campaign;
mod error;
mod rng;

const MAX_SEQUENCE_WORDS: usize = 65_536;
const MAX_RETAINED_FINDINGS: usize = 1_024;

pub use boundary::TargetPanicPayload;
pub use campaign::{
    CampaignSchedule, CampaignShard, CampaignVersion, CancellationBoundary, CaseIndices, CaseRange,
    ReplayCoordinates, CAMPAIGN_VERSION,
};
pub use error::{
    ConfigurationError, FuzzError, GeneratorError, InvariantError, ReductionError,
    ReferenceDisagreement, ReplayVersionError, ReportingError, SynchronizationError, WorkerError,
};
pub use report::{
    CheckIdentity, DivergenceClass, Finding, FindingKind, FuzzReport, FuzzRun, FuzzTarget,
    InstructionIdentity, OutcomeIdentity, ReductionOutcome, RunOutcome, SemanticFingerprint,
};
pub use sweep::{ppu_decode_partition, spu_decode_partition, DecodePanic, DecodeSweepReport};

/// Reusable configuration shared by all fuzz engines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FuzzConfig {
    /// Version of the serialized campaign and generator behavior.
    pub campaign_version: CampaignVersion,
    /// Master seed.
    pub seed: u64,
    /// Schedule for the campaign.
    pub schedule: CampaignSchedule,
    /// Maximum number of detailed findings retained in memory.
    pub max_findings: u32,
    /// Instruction count used by sequence engines.
    pub sequence_words: u32,
}

impl Default for FuzzConfig {
    fn default() -> Self {
        Self {
            campaign_version: CAMPAIGN_VERSION,
            seed: 1,
            schedule: CampaignSchedule::default(),
            max_findings: 20,
            sequence_words: 32,
        }
    }
}

impl FuzzConfig {
    /// Lists this invocation's stable case indices.
    pub fn case_indices(self) -> Result<CaseIndices, ConfigurationError> {
        self.validate_version()?;
        self.schedule.case_indices()
    }

    pub(crate) fn validate(self, sequence_limit: Option<usize>) -> Result<(), ConfigurationError> {
        self.validate_version()?;
        self.schedule.validate()?;
        if let Some(maximum) = sequence_limit {
            if self.sequence_words == 0 {
                return Err(ConfigurationError::ZeroSequenceWords);
            }
            if self.sequence_words as usize > maximum {
                return Err(ConfigurationError::SequenceTooLong {
                    requested: self.sequence_words as usize,
                    maximum,
                });
            }
        }
        if self.max_findings as usize > MAX_RETAINED_FINDINGS {
            return Err(ConfigurationError::TooManyRetainedFindings {
                requested: self.max_findings as usize,
                maximum: MAX_RETAINED_FINDINGS,
            });
        }
        Ok(())
    }

    fn validate_version(self) -> Result<(), ConfigurationError> {
        if self.campaign_version != CAMPAIGN_VERSION {
            return Err(ConfigurationError::UnsupportedCampaignVersion {
                found: self.campaign_version,
                supported: CAMPAIGN_VERSION,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/lib_tests.rs"]
mod tests;
