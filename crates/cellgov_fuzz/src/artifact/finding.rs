//! The finding artifact, its schema version and errors, and its construction and validation.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::ppu_reference::PpuReferenceError;
use crate::reduce::ReductionMeasure;
use crate::report::{Finding, FindingKind, FuzzReport};
use crate::spu_reference::SpuReferenceError;
use crate::{
    ConfigurationError, FuzzConfig, FuzzError, FuzzTarget, ReplayVersionError, TargetPanicPayload,
};

use super::reference::ArtifactReference;
use super::schema::{
    ArtifactCase, ArtifactCoverage, ArtifactExecutionPolicy, ArtifactFingerprint,
    ArtifactObservation, ArtifactReduction, ArtifactStateSource,
};

/// Schema version a finding artifact carries.
pub const FINDING_ARTIFACT_VERSION: u32 = 2;

/// Portable, versioned record for one retained finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FuzzFindingArtifact {
    /// Artifact schema version.
    pub schema_version: u32,
    /// Complete engine configuration for this campaign.
    pub campaign: FuzzConfig,
    /// Host-side budget and selection settings.
    pub execution: ArtifactExecutionPolicy,
    /// Original case, independent of any reduction result.
    pub original: ArtifactCase,
    /// Finding category.
    pub finding_kind: String,
    /// Stable semantic fingerprint.
    pub fingerprint: ArtifactFingerprint,
    /// Optional independent source, separate from the local checks.
    pub reference: ArtifactReference,
    /// Case observation, when execution reached semantic classification.
    pub observation: Option<ArtifactObservation>,
    /// Coverage context from the containing engine run.
    pub coverage: ArtifactCoverage,
    /// Reduction state; it never replaces [`Self::original`].
    pub reduction: ArtifactReduction,
    /// Typed target panic payload, if one occurred.
    pub panic_payload: Option<TargetPanicPayload>,
    /// Exact command tokens that replay this artifact at its written path.
    pub replay_command: Vec<String>,
}

/// Artifact construction or validation failure.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    /// The JSON did not parse.
    #[error("finding artifact JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    /// This build does not support the artifact schema.
    #[error("finding artifact version {found} is unsupported; expected {supported}")]
    Version {
        /// Version read from the artifact.
        found: u32,
        /// Version this build accepts.
        supported: u32,
    },
    /// Generator version is incompatible.
    #[error("finding artifact replay version: {0}")]
    ReplayVersion(#[from] ReplayVersionError),
    /// Campaign settings contradict the recorded case.
    #[error("finding artifact configuration: {0}")]
    Configuration(#[from] ConfigurationError),
    /// A required field or relationship is invalid.
    #[error("finding artifact is invalid: {0}")]
    Invalid(&'static str),
    /// Replay command cannot represent a non-UTF-8 path.
    #[error("finding artifact path is not valid UTF-8")]
    NonUtf8Path,
    /// PPU reference source is invalid.
    #[error("finding artifact PPU reference: {0}")]
    PpuReference(#[from] PpuReferenceError),
    /// SPU reference source is invalid.
    #[error("finding artifact SPU reference: {0}")]
    SpuReference(#[from] SpuReferenceError),
}

/// A replay that could not prove the original fingerprint.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactReplayError {
    /// Source artifact or generator version is incompatible.
    #[error("finding replay artifact: {0}")]
    Artifact(#[from] ArtifactError),
    /// The same case no longer produces this fingerprint and original word stream.
    #[error("finding replay did not reproduce the original fingerprint at case {case_index}")]
    NotReproduced {
        /// Original case index.
        case_index: u64,
    },
    /// The artifact records no reduced case to replay.
    #[error("finding artifact holds no reduced case")]
    NoReducedCase,
    /// The engine failed while it replayed the case; the finding did not change.
    #[error("finding replay harness failed: {source}")]
    HarnessFailure {
        /// Engine failure.
        #[source]
        source: FuzzError,
    },
    /// The recorded independent source no longer replays to a match.
    #[error("finding replay independent reference differs")]
    ReferenceMismatch,
    /// PPU reference execution failed.
    #[error("finding replay PPU reference: {0}")]
    PpuReference(#[from] PpuReferenceError),
    /// SPU reference execution failed.
    #[error("finding replay SPU reference: {0}")]
    SpuReference(#[from] SpuReferenceError),
}

impl FuzzFindingArtifact {
    /// Captures one finding and its containing run without replacing the original case.
    ///
    /// # Errors
    ///
    /// Refuses an incompatible configuration, source, or replay path.
    pub fn from_finding(
        campaign: FuzzConfig,
        execution: ArtifactExecutionPolicy,
        report: &FuzzReport,
        finding: &Finding,
        reference: ArtifactReference,
        artifact_path: &Path,
    ) -> Result<Self, ArtifactError> {
        // [Klees2018 p:2126 s:3 Overview and Experimental Setup] A result is
        // interpretable only with its parameters stated, so the artifact keeps
        // them beside the finding.
        let path = artifact_path.to_str().ok_or(ArtifactError::NonUtf8Path)?;
        let artifact = Self {
            schema_version: FINDING_ARTIFACT_VERSION,
            campaign,
            execution,
            original: ArtifactCase {
                replay: finding.replay,
                words: finding.original_words.clone(),
                state_source: ArtifactStateSource::VersionedGenerator,
            },
            finding_kind: format!("{:?}", finding.kind),
            fingerprint: ArtifactFingerprint::from(&finding.fingerprint),
            reference,
            observation: finding.observation.as_ref().map(ArtifactObservation::from),
            coverage: ArtifactCoverage::from(report),
            reduction: ArtifactReduction::from(&finding.reduction),
            panic_payload: finding.panic_payload.clone(),
            replay_command: vec![
                "cellgov".into(),
                "dev".into(),
                "fuzz".into(),
                "replay".into(),
                "--artifact".into(),
                path.into(),
            ],
        };
        artifact.validate()?;
        Ok(artifact)
    }

    /// Reports whether `other` records the same finding as this artifact.
    ///
    /// A finding's identity excludes:
    ///
    /// - the host budget,
    /// - the campaign range,
    /// - the coverage context,
    /// - the reduction state,
    /// - the replay path.
    #[must_use]
    pub fn describes_same_finding(&self, other: &Self) -> bool {
        self.schema_version == other.schema_version
            && self.original == other.original
            && self.finding_kind == other.finding_kind
            && self.fingerprint == other.fingerprint
            && self.reference == other.reference
            && self.observation == other.observation
            && self.panic_payload == other.panic_payload
    }

    /// Parses and validates a versioned artifact.
    ///
    /// # Errors
    ///
    /// Refuses unsupported schemas, incompatible generator versions, or inconsistent evidence.
    pub fn parse_json(json: &str) -> Result<Self, ArtifactError> {
        let artifact: Self = serde_json::from_str(json)?;
        artifact.validate()?;
        Ok(artifact)
    }

    /// Validates the schema and every replay relationship before any target executes.
    ///
    /// # Errors
    ///
    /// Refuses a changed generator version, mismatched source, or contradictory case.
    pub fn validate(&self) -> Result<(), ArtifactError> {
        if self.schema_version != FINDING_ARTIFACT_VERSION {
            return Err(ArtifactError::Version {
                found: self.schema_version,
                supported: FINDING_ARTIFACT_VERSION,
            });
        }
        self.original.replay.validate()?;
        self.campaign
            .validate_for_target(self.original.replay.target)?;
        if self.execution.workers == 0 || self.execution.deadline_ms == Some(0) {
            return Err(ArtifactError::Invalid("host execution bounds are invalid"));
        }
        self.reference.validate(self.original.replay.target)?;
        if self.campaign.campaign_version != self.original.replay.campaign_version
            || self.campaign.seed != self.original.replay.seed
            || self.campaign.strategy != self.original.replay.strategy
            || self.campaign.sequence_words != self.original.replay.sequence_words
            || self.fingerprint.target != self.original.replay.target
        {
            return Err(ArtifactError::Invalid(
                "campaign and original replay differ",
            ));
        }
        let offset = self
            .original
            .replay
            .case_index
            .checked_sub(self.campaign.schedule.cases.first)
            .ok_or(ArtifactError::Invalid("case precedes the campaign range"))?;
        if offset >= self.campaign.schedule.cases.count
            || self
                .campaign
                .schedule
                .cancellation
                .is_some_and(|stop| offset >= stop.0)
            || offset % u64::from(self.campaign.schedule.shard.count)
                != u64::from(self.campaign.schedule.shard.index)
        {
            return Err(ArtifactError::Invalid(
                "case is outside the selected campaign",
            ));
        }
        // An instruction engine decodes one word per case. A sequence engine
        // counts decoded words, at most `sequence_words` per case: the PPU
        // executor fetches at most one word per generated word and the SPU
        // executor runs under a `sequence_words` budget.
        let decoded_bound = match self.original.replay.target {
            FuzzTarget::PpuInstruction | FuzzTarget::SpuInstruction => self.coverage.cases,
            FuzzTarget::PpuSequence | FuzzTarget::SpuSequence => self
                .coverage
                .cases
                .saturating_mul(u64::from(self.campaign.sequence_words)),
        };
        if self.finding_kind.is_empty()
            || self.fingerprint.check.is_empty()
            || self.fingerprint.divergence.is_empty()
            || self
                .coverage
                .finding_counts
                .get(&self.finding_kind)
                .copied()
                .unwrap_or(0)
                == 0
            || self.coverage.cases == 0
            || self.coverage.decoded > decoded_bound
            || self.coverage.eligible > self.coverage.cases
        {
            return Err(ArtifactError::Invalid(
                "finding or coverage context is inconsistent",
            ));
        }
        if self.original.words.is_empty()
            && self.finding_kind != format!("{:?}", FindingKind::TargetPanic)
        {
            return Err(ArtifactError::Invalid(
                "non-panic finding has no original words",
            ));
        }
        if let ArtifactReduction::Reduced { words } = &self.reduction {
            if words.is_empty()
                || ReductionMeasure::of(words) >= ReductionMeasure::of(&self.original.words)
            {
                return Err(ArtifactError::Invalid(
                    "reduction is not a smaller nonempty case",
                ));
            }
        }
        if self.replay_command.len() != 6
            || self.replay_command[..5] != ["cellgov", "dev", "fuzz", "replay", "--artifact"]
            || self.replay_command[5].is_empty()
        {
            return Err(ArtifactError::Invalid("replay command is incomplete"));
        }
        Ok(())
    }
}
