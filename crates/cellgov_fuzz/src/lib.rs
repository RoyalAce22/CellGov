//! Deterministic fuzz engines for the PPU and SPU interpreters; callers provide host policy and campaign configuration.

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod artifact;
pub mod decoder_manifest;
pub mod evaluation;
pub mod ppu;
pub mod ppu_paths;
pub mod ppu_reference;
pub mod ppu_sequences;
pub mod raw_decode;
pub mod reduce;
pub mod reference;
pub mod report;
pub mod semantic_sweep;
pub mod spu;
pub mod spu_reference;
pub mod sweep;

mod boundary;
mod campaign;
mod case;
mod error;
mod parameters;
mod retention;
mod rng;
mod seeded;

const MAX_SEQUENCE_WORDS: usize = 65_536;
const MAX_RETAINED_FINDINGS: usize = 1_024;

pub use boundary::TargetPanicPayload;
pub use campaign::{
    CampaignSchedule, CampaignShard, CampaignVersion, CancellationBoundary, CaseIndices, CaseRange,
    GenerationStrategy, ReplayCoordinates, CAMPAIGN_VERSION,
};
pub use case::{CaseAssessment, CaseEligibility, CaseFeature, EligibilityReason};
pub use error::{
    ConfigurationError, FuzzError, GeneratorError, InvariantError, ReductionError,
    ReferenceDisagreement, ReplayVersionError, ReportingError, SynchronizationError, WorkerError,
};
pub use parameters::ParameterStream;
pub use report::{
    CheckIdentity, DivergenceClass, Finding, FindingKind, FuzzReport, FuzzRun, FuzzTarget,
    InstructionIdentity, OutcomeIdentity, ReductionOutcome, RunOutcome, SemanticFingerprint,
};
pub use retention::{
    BoundaryClass, CampaignDistribution, CrossReferenceAsymmetry, EvaluationDistribution,
    EvaluationError, ExplorationPolicy, OperandAliasClass, RetainedCase, RetainedCases,
    RetentionClass, RetentionConfig, RetentionConfigError, RetentionDecision, SemanticObservation,
    StateTransitionClass, TrialMetrics,
};
pub use sweep::{
    finite_partition_bounds, ppu_decode_partition, reduce_finite_results, spu_decode_partition,
    sweep_finite, DecodePanic, DecodeSweepReport, FiniteCase, FinitePartitionError,
    FiniteSweepError, FiniteSweepReport, FiniteVerdict,
};

/// Reusable configuration shared by all fuzz engines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct FuzzConfig {
    /// Version of the serialized campaign and generator behavior.
    pub campaign_version: CampaignVersion,
    /// Master seed.
    pub seed: u64,
    /// Selects how the campaign constructs input.
    pub strategy: GenerationStrategy,
    /// Schedule for the campaign.
    pub schedule: CampaignSchedule,
    /// Case-retention limits, weights, and exploration policy.
    pub retention: RetentionConfig,
    /// Maximum number of detailed findings retained in memory.
    pub max_findings: u32,
    /// Instruction count used by sequence engines.
    pub sequence_words: u32,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FuzzConfigArtifact {
    campaign_version: CampaignVersion,
    seed: u64,
    #[serde(default)]
    strategy: Option<GenerationStrategy>,
    schedule: CampaignSchedule,
    #[serde(default)]
    retention: Option<RetentionConfig>,
    max_findings: u32,
    sequence_words: u32,
}

impl<'de> serde::Deserialize<'de> for FuzzConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let artifact = <FuzzConfigArtifact as serde::Deserialize>::deserialize(deserializer)?;
        let strategy = match (artifact.campaign_version, artifact.strategy) {
            (_, Some(strategy)) => strategy,
            (CampaignVersion(1), None) => GenerationStrategy::RawWords,
            (_, None) => return Err(serde::de::Error::missing_field("strategy")),
        };
        let retention = match (artifact.campaign_version, artifact.retention) {
            (_, Some(retention)) => retention,
            (CAMPAIGN_VERSION, None) => return Err(serde::de::Error::missing_field("retention")),
            (_, None) => RetentionConfig::default(),
        };
        Ok(Self {
            campaign_version: artifact.campaign_version,
            seed: artifact.seed,
            strategy,
            schedule: artifact.schedule,
            retention,
            max_findings: artifact.max_findings,
            sequence_words: artifact.sequence_words,
        })
    }
}

impl Default for FuzzConfig {
    fn default() -> Self {
        Self {
            campaign_version: CAMPAIGN_VERSION,
            seed: 1,
            strategy: GenerationStrategy::Structured,
            schedule: CampaignSchedule::default(),
            retention: RetentionConfig::default(),
            max_findings: 20,
            sequence_words: 32,
        }
    }
}

impl FuzzConfig {
    /// Checks a campaign before the host schedules any interpreter work.
    ///
    /// # Errors
    ///
    /// Refuses incompatible versions, ranges, bounds, or retention settings.
    pub fn validate_for_target(self, target: FuzzTarget) -> Result<(), ConfigurationError> {
        let sequence_limit = match target {
            FuzzTarget::PpuInstruction | FuzzTarget::SpuInstruction => None,
            FuzzTarget::PpuSequence => Some(MAX_SEQUENCE_WORDS),
            FuzzTarget::SpuSequence => {
                Some((cellgov_spu::state::SPU_LS_SIZE / 4).min(MAX_SEQUENCE_WORDS))
            }
        };
        self.validate(sequence_limit)
    }

    /// Lists this invocation's stable case indices.
    pub fn case_indices(self) -> Result<CaseIndices, ConfigurationError> {
        self.validate_version()?;
        self.schedule.case_indices()
    }

    pub(crate) fn validate(self, sequence_limit: Option<usize>) -> Result<(), ConfigurationError> {
        self.validate_version()?;
        self.schedule.validate()?;
        self.retention
            .validate()
            .map_err(|source| ConfigurationError::Retention { source })?;
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

#[cfg(test)]
#[path = "tests/retention_tests.rs"]
mod retention_tests;

#[cfg(test)]
#[path = "tests/module_coverage_tests.rs"]
mod module_coverage_tests;
