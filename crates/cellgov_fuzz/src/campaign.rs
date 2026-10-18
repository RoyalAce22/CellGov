//! Versioned campaign plans and stable case coordinates.

use serde::{Deserialize, Serialize};

use crate::{ConfigurationError, FuzzTarget, ReplayVersionError};

/// Version of the deterministic case-to-input mapping.
pub const CAMPAIGN_VERSION: CampaignVersion = CampaignVersion(4);

/// Revision of one target and strategy's case-to-input mapping within a
/// campaign version.
///
/// A change to one mapping raises that mapping's revision and leaves the
/// others' replay records valid. Revision 0 is the mapping the
/// campaign version started with, and a replay record that names no
/// revision names 0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GeneratorRevision(pub u32);

impl GeneratorRevision {
    fn is_initial(&self) -> bool {
        self.0 == 0
    }
}

impl std::fmt::Display for GeneratorRevision {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The revision of the generator `target` runs under `strategy`, within
/// [`CAMPAIGN_VERSION`].
///
/// Both registries grew under version 4, which moves every structured
/// draw, and the SPU sequence engine gained its interactions and scripted
/// world. The PPU raw-word generators draw whole words with no registry,
/// and the committed regressions recorded from them still replay, so they
/// keep revision 0. The SPU raw-word mappings are not shown unchanged, so
/// they take revision 1 with the rest.
pub const fn generator_revision(
    target: FuzzTarget,
    strategy: GenerationStrategy,
) -> GeneratorRevision {
    match (target, strategy) {
        (FuzzTarget::PpuInstruction | FuzzTarget::PpuSequence, GenerationStrategy::RawWords) => {
            GeneratorRevision(0)
        }
        _ => GeneratorRevision(1),
    }
}

/// Specifies how a fuzz campaign constructs input.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationStrategy {
    /// Selects an interpreter-owned descriptor, then encodes typed operands.
    Structured,
    /// Generates complete words to test decoder robustness.
    #[default]
    RawWords,
}

/// Version of a serialized campaign and its generator behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CampaignVersion(pub u32);

impl std::fmt::Display for CampaignVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A contiguous range in the campaign's stable case-index space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseRange {
    /// First case index in the range.
    pub first: u64,
    /// Number of cases in the range.
    pub count: u64,
}

/// One deterministic partition of a campaign range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignShard {
    /// Zero-based shard number.
    pub index: u32,
    /// Total number of shards.
    pub count: u32,
}

impl CampaignShard {
    /// A schedule that assigns every case to one runner.
    pub const ALL: Self = Self { index: 0, count: 1 };
}

/// A deterministic stop point measured from the start of the full range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CancellationBoundary(pub u64);

/// Schedules cases without changing their replay identity.
///
/// Each case's stable index fixes its replay identity. A shard takes every
/// count-th index by stride, the way parallel testers take sizes i, i+k,
/// i+2k. The shards together cover exactly the sequential run's cases.
/// [Krook2023 p:4 s:4 Design and Implementation (testing loop)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignSchedule {
    /// Stable range of case indices.
    pub cases: CaseRange,
    /// Deterministic partition assigned to this invocation.
    pub shard: CampaignShard,
    /// Stop before this offset in the full range, when present.
    pub cancellation: Option<CancellationBoundary>,
}

impl Default for CampaignSchedule {
    fn default() -> Self {
        Self {
            cases: CaseRange {
                first: 0,
                count: 1_000_000,
            },
            shard: CampaignShard::ALL,
            cancellation: None,
        }
    }
}

impl CampaignSchedule {
    pub(crate) fn validate(self) -> Result<(), ConfigurationError> {
        if self.cases.count == 0 {
            return Err(ConfigurationError::ZeroIterations);
        }
        if self.cases.first.checked_add(self.cases.count - 1).is_none() {
            return Err(ConfigurationError::CaseRangeOverflow {
                first: self.cases.first,
                count: self.cases.count,
            });
        }
        if self.shard.count == 0 || self.shard.index >= self.shard.count {
            return Err(ConfigurationError::InvalidShard {
                index: self.shard.index,
                count: self.shard.count,
            });
        }
        if let Some(CancellationBoundary(offset)) = self.cancellation {
            if offset > self.cases.count {
                return Err(ConfigurationError::CancellationOutOfRange {
                    offset,
                    count: self.cases.count,
                });
            }
        }
        Ok(())
    }

    /// Lists this shard's stable case indices.
    pub fn case_indices(self) -> Result<CaseIndices, ConfigurationError> {
        self.validate()?;
        let scheduled = self
            .cancellation
            .map_or(self.cases.count, |boundary| boundary.0);
        Ok(CaseIndices {
            next: self.cases.first,
            remaining: scheduled,
            offset: 0,
            shard: self.shard,
        })
    }

    /// Reports whether cancellation excludes part of the declared range.
    pub fn is_cancelled(self) -> bool {
        self.cancellation
            .is_some_and(|boundary| boundary.0 < self.cases.count)
    }
}

/// Iterator over one shard's stable case indices.
#[derive(Debug, Clone)]
pub struct CaseIndices {
    next: u64,
    remaining: u64,
    offset: u64,
    shard: CampaignShard,
}

impl Iterator for CaseIndices {
    type Item = u64;

    fn next(&mut self) -> Option<Self::Item> {
        while self.remaining != 0 {
            let index = self.next;
            let offset = self.offset;
            self.remaining -= 1;
            self.offset += 1;
            if self.remaining != 0 {
                self.next += 1;
            }
            if offset % u64::from(self.shard.count) == u64::from(self.shard.index) {
                return Some(index);
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, usize::try_from(self.remaining).ok())
    }
}

/// Exact coordinates needed to reconstruct one generated case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayCoordinates {
    /// Version of the deterministic generator behavior.
    pub campaign_version: CampaignVersion,
    /// Revision of the mapping the target runs under its strategy.
    #[serde(skip_serializing_if = "GeneratorRevision::is_initial")]
    pub generator_revision: GeneratorRevision,
    /// Engine that generates the case.
    pub target: FuzzTarget,
    /// Selects how this case constructs input.
    pub strategy: GenerationStrategy,
    /// Master campaign seed.
    pub seed: u64,
    /// Case index within the campaign's stable coordinate space.
    pub case_index: u64,
    /// Instruction count used when the target is a sequence engine.
    pub sequence_words: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayCoordinatesArtifact {
    campaign_version: CampaignVersion,
    #[serde(default)]
    generator_revision: GeneratorRevision,
    target: FuzzTarget,
    #[serde(default)]
    strategy: Option<GenerationStrategy>,
    seed: u64,
    case_index: u64,
    sequence_words: u32,
}

impl<'de> Deserialize<'de> for ReplayCoordinates {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let artifact = ReplayCoordinatesArtifact::deserialize(deserializer)?;
        let strategy = match (artifact.campaign_version, artifact.strategy) {
            (_, Some(strategy)) => strategy,
            (CampaignVersion(1), None) => GenerationStrategy::RawWords,
            (_, None) => return Err(serde::de::Error::missing_field("strategy")),
        };
        Ok(Self {
            campaign_version: artifact.campaign_version,
            generator_revision: artifact.generator_revision,
            target: artifact.target,
            strategy,
            seed: artifact.seed,
            case_index: artifact.case_index,
            sequence_words: artifact.sequence_words,
        })
    }
}

impl ReplayCoordinates {
    pub(crate) fn new(
        target: FuzzTarget,
        strategy: GenerationStrategy,
        seed: u64,
        case_index: u64,
        sequence_words: u32,
    ) -> Self {
        Self {
            campaign_version: CAMPAIGN_VERSION,
            generator_revision: generator_revision(target, strategy),
            target,
            strategy,
            seed,
            case_index,
            sequence_words,
        }
    }

    /// Validates the campaign version and the target's generator revision.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayVersionError`] if the campaign version differs from
    /// [`CAMPAIGN_VERSION`], or the generator revision from the one
    /// [`generator_revision`] gives the target and strategy.
    pub fn validate(self) -> Result<(), ReplayVersionError> {
        if self.campaign_version != CAMPAIGN_VERSION {
            return Err(ReplayVersionError::Campaign {
                found: self.campaign_version,
                supported: CAMPAIGN_VERSION,
            });
        }
        let supported = generator_revision(self.target, self.strategy);
        if self.generator_revision != supported {
            return Err(ReplayVersionError::GeneratorRevision {
                target: self.target,
                strategy: self.strategy,
                found: self.generator_revision,
                supported,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/campaign_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/generator_revision_tests.rs"]
mod revision_tests;
