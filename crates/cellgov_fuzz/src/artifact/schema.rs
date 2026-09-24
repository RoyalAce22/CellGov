//! The serialized records an artifact is made of, and their conversions from the engine's types.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::reduce::ReductionPolicy;
use crate::report::{FuzzReport, ReductionOutcome, SemanticFingerprint};
use crate::{FuzzTarget, ReplayCoordinates};

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
    /// Decoded cases, or decoded instruction words for a sequence engine.
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
