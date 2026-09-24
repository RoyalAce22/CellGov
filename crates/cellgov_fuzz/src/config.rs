//! The campaign configuration every engine takes, its serialized form,
//! and its validation.

use crate::campaign::{
    CampaignSchedule, CampaignVersion, CaseIndices, GenerationStrategy, CAMPAIGN_VERSION,
};
use crate::error::ConfigurationError;
use crate::report::{FuzzRun, FuzzTarget};
use crate::retention::RetentionConfig;
use crate::{ppu, spu};

pub(crate) const MAX_SEQUENCE_WORDS: usize = 65_536;

/// Most detailed findings one campaign may retain in memory.
pub const MAX_RETAINED_FINDINGS: u32 = 1_024;

/// Detailed findings a campaign retains when the caller names no limit.
pub const DEFAULT_MAX_FINDINGS: u32 = 20;

/// Instruction words per case a sequence engine generates when the caller
/// names no count.
pub const DEFAULT_SEQUENCE_WORDS: u32 = 32;

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
            max_findings: DEFAULT_MAX_FINDINGS,
            sequence_words: DEFAULT_SEQUENCE_WORDS,
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
        if self.max_findings > MAX_RETAINED_FINDINGS {
            return Err(ConfigurationError::TooManyRetainedFindings {
                requested: self.max_findings as usize,
                maximum: MAX_RETAINED_FINDINGS as usize,
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

impl FuzzTarget {
    /// Runs this target's engine over `config`.
    pub fn run(self, config: FuzzConfig) -> FuzzRun {
        match self {
            Self::PpuInstruction => ppu::run_instructions(config),
            Self::PpuSequence => ppu::run_sequences(config),
            Self::SpuInstruction => spu::run_instructions(config),
            Self::SpuSequence => spu::run_sequences(config),
        }
    }

    /// Whether this target's cases are instruction sequences, the only
    /// cases [`FuzzConfig::sequence_words`] sizes.
    pub const fn generates_sequences(self) -> bool {
        matches!(self, Self::PpuSequence | Self::SpuSequence)
    }
}

#[cfg(test)]
#[path = "tests/config_tests.rs"]
mod tests;
