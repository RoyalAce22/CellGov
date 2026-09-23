//! Versioned finding evidence with exact original-case replay.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::ppu_reference::{parse_reference_json as parse_ppu_reference, PpuReferenceError};
use crate::reduce::{evaluate_case, ReductionMeasure, ReductionPolicy};
use crate::report::{
    Finding, FindingKind, FuzzReport, FuzzRun, ReductionOutcome, RunOutcome, SemanticFingerprint,
};
use crate::spu_reference::{parse_reference_json as parse_spu_reference, SpuReferenceError};
use crate::{
    ppu, spu, CampaignSchedule, CampaignShard, CaseRange, ConfigurationError, FuzzConfig,
    FuzzError, FuzzTarget, ReplayCoordinates, ReplayVersionError, TargetPanicPayload,
};

/// Schema version a finding artifact carries.
pub const FINDING_ARTIFACT_VERSION: u32 = 2;

/// Named independent source retained inside an artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "source_json",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ArtifactReference {
    /// Checks only interpreter-owned local contracts.
    Local,
    /// Includes a validated PPU vector or hardware capture.
    Ppu(String),
    /// Includes a validated SPU vector or hardware capture.
    Spu(String),
}

impl ArtifactReference {
    /// Canonicalizes a versioned PPU vector or capture for portable replay.
    ///
    /// # Errors
    ///
    /// Refuses invalid source content or provenance.
    pub fn ppu(json: &str) -> Result<Self, ArtifactError> {
        Ok(Self::Ppu(serde_json::to_string(&parse_ppu_reference(
            json,
        )?)?))
    }

    /// Canonicalizes a versioned SPU vector or capture for portable replay.
    ///
    /// # Errors
    ///
    /// Refuses invalid source content or provenance.
    pub fn spu(json: &str) -> Result<Self, ArtifactError> {
        Ok(Self::Spu(serde_json::to_string(&parse_spu_reference(
            json,
        )?)?))
    }

    fn validate(&self, target: FuzzTarget) -> Result<(), ArtifactError> {
        match self {
            Self::Local => Ok(()),
            Self::Ppu(json)
                if matches!(target, FuzzTarget::PpuInstruction | FuzzTarget::PpuSequence) =>
            {
                let source = parse_ppu_reference(json)?;
                (serde_json::to_string(&source)? == *json)
                    .then_some(())
                    .ok_or(ArtifactError::Invalid("PPU source is not canonical"))
            }
            Self::Spu(json)
                if matches!(target, FuzzTarget::SpuInstruction | FuzzTarget::SpuSequence) =>
            {
                let source = parse_spu_reference(json)?;
                (serde_json::to_string(&source)? == *json)
                    .then_some(())
                    .ok_or(ArtifactError::Invalid("SPU source is not canonical"))
            }
            Self::Ppu(_) | Self::Spu(_) => Err(ArtifactError::Invalid(
                "reference interpreter differs from target",
            )),
        }
    }
}

/// Typed semantic classification rendered without process-dependent text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactFingerprint {
    /// Engine that produced the finding.
    pub target: FuzzTarget,
    /// Interpreter-owned instruction kind, when decode reached one.
    pub instruction_kind: Option<String>,
    /// Named check or target boundary.
    pub check: String,
    /// Classified difference or refusal.
    pub divergence: String,
    /// Typed terminal outcome, when relevant.
    pub outcome: Option<String>,
    /// Guest-visible effect class, when relevant.
    pub effect: Option<String>,
}

impl From<&SemanticFingerprint> for ArtifactFingerprint {
    fn from(source: &SemanticFingerprint) -> Self {
        Self {
            target: source.target,
            instruction_kind: source.instruction_kind.map(|kind| format!("{kind:?}")),
            check: format!("{:?}", source.check),
            divergence: format!("{:?}", source.divergence),
            outcome: source.outcome.map(|outcome| format!("{outcome:?}")),
            effect: source.effect.map(|effect| format!("{effect:?}")),
        }
    }
}

/// Initial generated input needed to replay the finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactCase {
    /// Versioned generator coordinates, including seed and index.
    pub replay: ReplayCoordinates,
    /// Original unreduced instruction words.
    pub words: Vec<u32>,
    /// Provenance of the initial architectural state.
    pub state_source: ArtifactStateSource,
}

/// Provenance of a case's initial architectural state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStateSource {
    /// The generator version and seed in [`ReplayCoordinates`] recreate the state.
    VersionedGenerator,
}

/// One case observation retained even when novelty scheduling drops the case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactObservation {
    /// First executed interpreter kind, when one exists.
    pub first_instruction_kind: Option<String>,
    /// Executed interpreter kinds in stable order.
    pub instruction_kinds: Vec<String>,
    /// Operand alias and boundary class.
    pub operands: String,
    /// Eligibility for the selected check.
    pub eligibility: String,
    /// Terminal outcome, when one exists.
    pub outcome: Option<String>,
    /// Architectural state-transition class.
    pub state_transition: String,
    /// Guest-visible effect classes.
    pub effects: Vec<String>,
    /// Generated boundary classes.
    pub boundaries: Vec<String>,
    /// Number of executed instructions.
    pub sequence_depth: u64,
    /// Cross-reference asymmetry class.
    pub asymmetry: String,
}

impl From<&crate::SemanticObservation> for ArtifactObservation {
    fn from(source: &crate::SemanticObservation) -> Self {
        Self {
            first_instruction_kind: source
                .first_instruction_kind
                .map(|kind| format!("{kind:?}")),
            instruction_kinds: source
                .instruction_kinds
                .iter()
                .map(|kind| format!("{kind:?}"))
                .collect(),
            operands: format!("{:?}", source.operands),
            eligibility: format!("{:?}", source.eligibility),
            outcome: source.outcome.map(|outcome| format!("{outcome:?}")),
            state_transition: format!("{:?}", source.state_transition),
            effects: source
                .effects
                .iter()
                .map(|effect| format!("{effect:?}"))
                .collect(),
            boundaries: source
                .boundaries
                .iter()
                .map(|boundary| format!("{boundary:?}"))
                .collect(),
            sequence_depth: source.sequence_depth,
            asymmetry: format!("{:?}", source.asymmetry),
        }
    }
}

/// Coverage at the point the engine retained the finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactCoverage {
    /// Cases the containing engine run considered.
    pub cases: u64,
    /// Successfully decoded cases.
    pub decoded: u64,
    /// Cases eligible for their semantic check.
    pub eligible: u64,
    /// Cases the target refused as unmodeled.
    pub unsupported: u64,
    /// Cases with undefined semantics.
    pub undefined: u64,
    /// Count of every finding kind, retained or not.
    pub finding_counts: BTreeMap<String, u64>,
}

/// Host execution settings that affect budgeting but not case identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactExecutionPolicy {
    /// Worker count the caller requested for the campaign.
    pub workers: u32,
    /// Host deadline in milliseconds, when supplied.
    pub deadline_ms: Option<u64>,
    /// Whether the caller requested progress reporting.
    pub progress: bool,
    /// Check policy the caller selected.
    pub check: ArtifactCheckSelection,
    /// Requested reduction behavior.
    pub reduction: ArtifactReductionRequest,
}

/// Check policy this artifact schema represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactCheckSelection {
    /// Run all interpreter-owned checks.
    All,
}

/// Requested reduction behavior, distinct from its recorded outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArtifactReductionRequest {
    /// Do not run a reducer.
    None,
    /// Reduce each finding while preserving its fingerprint.
    OnFinding {
        /// Candidate selection policy.
        policy: ReductionPolicy,
        /// Maximum candidate evaluations per finding.
        budget: u64,
    },
}

impl From<&FuzzReport> for ArtifactCoverage {
    fn from(report: &FuzzReport) -> Self {
        Self {
            cases: report.cases,
            decoded: report.decoded,
            eligible: report.eligible_cases,
            unsupported: report.unsupported_cases,
            undefined: report.undefined_cases,
            finding_counts: report
                .finding_counts
                .iter()
                .map(|(kind, count)| (format!("{kind:?}"), *count))
                .collect(),
        }
    }
}

/// Reduction state with the original case retained independently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArtifactReduction {
    /// No reducer ran.
    NotAttempted,
    /// Same-fingerprint reduction produced these case words.
    Reduced {
        /// Reduced case words; the artifact still retains the original.
        words: Vec<u32>,
    },
    /// Every candidate lost the finding; the original case is already minimal.
    Irreducible,
    /// Reduction failed; the artifact still replays the original case.
    Failed {
        /// The reducer refusal, rendered as text.
        reason: String,
    },
}

impl From<&ReductionOutcome> for ArtifactReduction {
    fn from(source: &ReductionOutcome) -> Self {
        match source {
            ReductionOutcome::NotAttempted => Self::NotAttempted,
            ReductionOutcome::Reduced(words) => Self::Reduced {
                words: words.clone(),
            },
            ReductionOutcome::Irreducible => Self::Irreducible,
            ReductionOutcome::Failed(error) => Self::Failed {
                reason: error.to_string(),
            },
        }
    }
}

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
        // [Klees2018 p:2123 s:Introduction] Evaluation records seed and timeout settings and samples performance across trials.
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
            || self.coverage.decoded > self.coverage.cases
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

    /// Replays this artifact through the selected interpreter engine.
    ///
    /// # Errors
    ///
    /// Refuses incompatible versions or any changed original fingerprint.
    pub fn replay(&self) -> Result<Finding, ArtifactReplayError> {
        self.replay_with(|config| match self.original.replay.target {
            FuzzTarget::PpuInstruction => ppu::run_instructions(config),
            FuzzTarget::PpuSequence => ppu::run_sequences(config),
            FuzzTarget::SpuInstruction => spu::run_instructions(config),
            FuzzTarget::SpuSequence => spu::run_sequences(config),
        })
    }

    /// Replays through a caller-supplied runner, behind the validation gate of [`Self::replay`].
    ///
    /// # Errors
    ///
    /// Refuses incompatible versions or a missing original fingerprint.
    pub fn replay_with(
        &self,
        run: impl FnOnce(FuzzConfig) -> FuzzRun,
    ) -> Result<Finding, ArtifactReplayError> {
        self.validate()?;
        self.check_independent_reference()?;
        let mut config = self.campaign;
        config.schedule = CampaignSchedule {
            cases: CaseRange {
                first: self.original.replay.case_index,
                count: 1,
            },
            shard: CampaignShard::ALL,
            cancellation: None,
        };
        let replay = run(config);
        if let RunOutcome::HarnessFailure(source) = replay.outcome {
            return Err(ArtifactReplayError::HarnessFailure { source });
        }
        // A target panic with a changed message is a different finding, so the payload compares
        // like the fingerprint. Engines never reduce, so the comparison skips the reduction and a
        // reduced artifact still replays its original case.
        for finding in replay.report.findings {
            if finding.replay == self.original.replay
                && finding.original_words == self.original.words
                && ArtifactFingerprint::from(&finding.fingerprint) == self.fingerprint
                && format!("{:?}", finding.kind) == self.finding_kind
                && finding.observation.as_ref().map(ArtifactObservation::from) == self.observation
                && finding.panic_payload == self.panic_payload
            {
                return Ok(finding);
            }
        }
        Err(ArtifactReplayError::NotReproduced {
            case_index: self.original.replay.case_index,
        })
    }

    /// Replays the reduced case words and requires the recorded finding identity.
    ///
    /// A reduced case keeps the original's generated state, so its observation
    /// may differ. Its kind, fingerprint and panic payload stay the same.
    ///
    /// # Errors
    ///
    /// Refuses:
    ///
    /// - an artifact without a reduced case
    /// - an incompatible version
    /// - an engine failure
    /// - reduced words that no longer reproduce the finding
    pub fn replay_reduced(&self) -> Result<Finding, ArtifactReplayError> {
        self.validate()?;
        let ArtifactReduction::Reduced { words } = &self.reduction else {
            return Err(ArtifactReplayError::NoReducedCase);
        };
        self.check_independent_reference()?;
        let replay = evaluate_case(
            self.original.replay.target,
            self.campaign,
            self.original.replay.case_index,
            words,
        );
        self.reduced_finding(replay)
    }

    /// Finds the recorded finding identity in a reduced-case run.
    fn reduced_finding(&self, replay: FuzzRun) -> Result<Finding, ArtifactReplayError> {
        if let RunOutcome::HarnessFailure(source) = replay.outcome {
            return Err(ArtifactReplayError::HarnessFailure { source });
        }
        replay
            .report
            .findings
            .into_iter()
            .find(|finding| {
                finding.replay == self.original.replay
                    && ArtifactFingerprint::from(&finding.fingerprint) == self.fingerprint
                    && format!("{:?}", finding.kind) == self.finding_kind
                    && finding.panic_payload == self.panic_payload
            })
            .ok_or(ArtifactReplayError::NotReproduced {
                case_index: self.original.replay.case_index,
            })
    }

    fn check_independent_reference(&self) -> Result<(), ArtifactReplayError> {
        match &self.reference {
            ArtifactReference::Local => Ok(()),
            ArtifactReference::Ppu(json) => {
                let source = parse_ppu_reference(json)?;
                let replay = crate::ppu_reference::replay_reference(&source)?;
                if replay.internal_divergence.is_some()
                    || replay.comparisons.is_empty()
                    || replay
                        .comparisons
                        .iter()
                        .any(|comparison| !comparison.is_match())
                {
                    return Err(ArtifactReplayError::ReferenceMismatch);
                }
                Ok(())
            }
            ArtifactReference::Spu(json) => {
                let source = parse_spu_reference(json)?;
                let replay = crate::spu_reference::replay_reference(&source)?;
                if !replay.comparison.is_match() {
                    return Err(ArtifactReplayError::ReferenceMismatch);
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/artifact_tests.rs"]
mod tests;
