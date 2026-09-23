//! Typed outcome records for every `dev fuzz` result, the exit status each
//! one maps to, and the one place that renders their terminal text.
//!
//! Identity stays in the records: callers compare a fingerprint or a case
//! index as a value, and rendered text carries none. [Chen2013 p:1 s:Abstract]

use std::collections::BTreeMap;
use std::path::PathBuf;

use cellgov_fuzz::artifact::{ArtifactFingerprint, ArtifactReduction};
use cellgov_fuzz::raw_decode::{RawDecodeStatus, RawDecoder};
use cellgov_fuzz::{FindingKind, FuzzTarget};

use crate::cli::exit::CommandExitCode;
use crate::cli::exit_codes;

/// Exit code: a deadline or `--cancel-after` ended the range before every
/// case ran, and the campaign retained no finding.
pub(crate) const EXIT_CANCELLED: i32 = exit_codes::command_specific(10);
/// Exit code: every scheduled case ran and none was eligible for its check.
pub(crate) const EXIT_NO_ELIGIBLE_CASES: i32 = exit_codes::command_specific(11);
/// Exit code: the engine failed inside the harness.
pub(crate) const EXIT_HARNESS_FAILURE: i32 = exit_codes::command_specific(12);
/// Exit code: the command could not store a finding's artifact, so it
/// printed the evidence.
pub(crate) const EXIT_EVIDENCE_NOT_STORED: i32 = exit_codes::command_specific(13);
/// Exit code: a retained finding's reduction failed; its artifact holds the
/// original case.
pub(crate) const EXIT_REDUCTION_FAILED: i32 = exit_codes::command_specific(14);
/// Exit code: a stored finding no longer reproduces at its case.
pub(crate) const EXIT_NOT_REPRODUCED: i32 = exit_codes::command_specific(15);

/// One retained finding's artifact, as the summary names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtifactRecord {
    /// Path the artifact write targeted.
    pub path: PathBuf,
    /// Original case index.
    pub case_index: u64,
    pub finding_kind: FindingKind,
    /// Stable fingerprint, as the artifact records it.
    pub fingerprint: ArtifactFingerprint,
    /// Reduction state stored with the finding.
    pub reduction: ArtifactReduction,
    /// Whether the artifact reached its path.
    pub stored: bool,
}

impl ArtifactRecord {
    /// Exact command that replays this artifact's original case.
    #[must_use]
    pub fn replay_command(&self) -> String {
        format!("cellgov dev fuzz replay --artifact {}", self.path.display())
    }
}

/// Counts of one generated campaign, accumulated over every worker run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct CampaignSummary {
    /// Cases considered, decode refusals included.
    pub cases: u64,
    /// Decoded cases, or decoded instruction words for a sequence engine.
    pub decoded: u64,
    /// Cases eligible for their semantic check.
    pub eligible: u64,
    /// Cases the target refused as unmodeled.
    pub unsupported: u64,
    /// Cases the architecture leaves undefined.
    pub undefined: u64,
    /// Findings by kind, retained or not.
    pub finding_counts: BTreeMap<FindingKind, u64>,
    /// Retained findings, in storage order.
    pub artifacts: Vec<ArtifactRecord>,
    /// Retained findings whose reduction failed.
    pub reductions_failed: u64,
    /// Whether the range ended before the campaign considered every case index.
    pub cancelled: bool,
}

impl CampaignSummary {
    /// Findings that fail the campaign; `Unsupported` and `Undefined` classify
    /// a case and count as none, as the engine's own outcome treats them.
    #[must_use]
    pub fn findings(&self) -> u64 {
        self.finding_counts
            .iter()
            .filter(|(kind, _)| !matches!(kind, FindingKind::Unsupported | FindingKind::Undefined))
            .fold(0u64, |total, (_, count)| total.saturating_add(*count))
    }
}

/// Terminal state of one generated campaign, in exit precedence order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CampaignOutcome {
    /// The command could not store a finding's artifact.
    EvidenceNotStored,
    /// The engine failed inside the harness.
    HarnessFailure,
    /// A retained finding's reduction failed.
    ReductionFailed,
    /// The campaign retained a finding or a target panic.
    Findings,
    /// Every case ran and none was eligible.
    NoEligibleCases,
    /// The range ended before every case ran.
    Cancelled,
    /// Every case ran clean.
    Clean,
}

impl CampaignOutcome {
    #[must_use]
    pub fn classify(
        summary: &CampaignSummary,
        evidence_not_stored: bool,
        harness_failure: bool,
    ) -> Self {
        if evidence_not_stored {
            Self::EvidenceNotStored
        } else if harness_failure {
            Self::HarnessFailure
        } else if summary.reductions_failed > 0 {
            Self::ReductionFailed
        } else if summary.findings() > 0 {
            Self::Findings
        } else if summary.cancelled {
            Self::Cancelled
        } else if summary.eligible == 0 {
            Self::NoEligibleCases
        } else {
            Self::Clean
        }
    }

    /// The documented exit status for this outcome.
    #[must_use]
    pub const fn exit_code(self) -> CommandExitCode {
        match self {
            Self::EvidenceNotStored => CommandExitCode::new(EXIT_EVIDENCE_NOT_STORED),
            Self::HarnessFailure => CommandExitCode::new(EXIT_HARNESS_FAILURE),
            Self::ReductionFailed => CommandExitCode::new(EXIT_REDUCTION_FAILED),
            Self::Findings => CommandExitCode::new(exit_codes::FAILED),
            Self::NoEligibleCases => CommandExitCode::new(EXIT_NO_ELIGIBLE_CASES),
            Self::Cancelled => CommandExitCode::new(EXIT_CANCELLED),
            Self::Clean => CommandExitCode::SUCCESS,
        }
    }
}

/// Progress after one batch of a generated campaign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CampaignProgress {
    /// Case indices considered so far.
    pub considered: u64,
    /// Case indices the campaign will consider.
    pub count: u64,
    /// Cases the workers processed so far.
    pub processed: u64,
}

/// One replayed artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplayOutcome {
    /// Original case index.
    pub case_index: u64,
    /// Whether the replay ran the reduced case.
    pub reduced: bool,
    /// Finding kind, as the engine reported it.
    pub finding_kind: String,
    /// Stable fingerprint, as the artifact records it.
    pub fingerprint: ArtifactFingerprint,
    /// Words the reproduced finding recorded.
    pub words: Vec<u32>,
}

impl ReplayOutcome {
    /// A reproduced finding is still a finding: the same status the campaign
    /// that stored it returned.
    #[must_use]
    pub const fn exit_code(&self) -> CommandExitCode {
        CommandExitCode::new(exit_codes::FAILED)
    }
}

/// One descriptor registry's semantic enumeration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SemanticSummary {
    /// Interpreter the registry belongs to.
    pub interpreter: &'static str,
    /// Instruction kinds the registry declares.
    pub kinds: usize,
    /// Kinds with an executable witness.
    pub witnesses: usize,
    /// Disagreements between the registry and its interpreter.
    pub findings: usize,
    /// Kinds the registry declares as refused.
    pub refusals: u64,
}

/// One raw decoder scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawSummary {
    /// Decoder scanned.
    pub decoder: RawDecoder,
    /// Completion state.
    pub status: RawDecodeStatus,
    /// Words scanned.
    pub processed: u64,
    /// Words in the selected domain.
    pub domain: u64,
    /// Words the decoder accepted.
    pub accepted: u64,
    /// Words the decoder refused.
    pub refused: u64,
    /// Words the decoder panicked on.
    pub panics: u64,
    /// Path the scan wrote the versioned result to.
    pub output: Option<PathBuf>,
}

impl RawSummary {
    /// The documented exit status for this scan.
    #[must_use]
    pub const fn exit_code(&self) -> CommandExitCode {
        if self.panics > 0 {
            CommandExitCode::new(exit_codes::FAILED)
        } else if matches!(self.status, RawDecodeStatus::Cancelled) {
            CommandExitCode::new(EXIT_CANCELLED)
        } else {
            CommandExitCode::SUCCESS
        }
    }
}

#[must_use]
pub(crate) fn render_campaign_progress(progress: CampaignProgress) -> String {
    format!(
        "fuzz: considered {} of {} case indices; processed {}",
        progress.considered, progress.count, progress.processed
    )
}

/// Renders a campaign's summary: one status line, then one line per artifact
/// with its exact replay command.
#[must_use]
pub(crate) fn render_campaign_summary(
    target: FuzzTarget,
    summary: &CampaignSummary,
    outcome: CampaignOutcome,
) -> String {
    let mut text = format!(
        "fuzz: {target:?} cases={} decoded={} eligible={} unsupported={} undefined={} findings={} reductions_failed={} cancelled={} outcome={outcome:?}\n",
        summary.cases,
        summary.decoded,
        summary.eligible,
        summary.unsupported,
        summary.undefined,
        summary.findings(),
        summary.reductions_failed,
        summary.cancelled,
    );
    for artifact in &summary.artifacts {
        let reduction = match &artifact.reduction {
            ArtifactReduction::NotAttempted => "not attempted".to_owned(),
            ArtifactReduction::Reduced { words } => format!("reduced to {} words", words.len()),
            ArtifactReduction::Irreducible => "irreducible".to_owned(),
            ArtifactReduction::Failed { reason } => format!("failed ({reason})"),
        };
        text.push_str(&format!(
            "fuzz: finding case={} kind={:?} check={} divergence={} reduction={reduction} artifact={} {}\n",
            artifact.case_index,
            artifact.finding_kind,
            artifact.fingerprint.check,
            artifact.fingerprint.divergence,
            if artifact.stored { "stored" } else { "not stored" },
            artifact.replay_command(),
        ));
    }
    text
}

#[must_use]
pub(crate) fn render_replay_outcome(outcome: &ReplayOutcome) -> String {
    format!(
        "fuzz replay: reproduced case={} reduced={} kind={} check={} divergence={} words={:?}\n",
        outcome.case_index,
        outcome.reduced,
        outcome.finding_kind,
        outcome.fingerprint.check,
        outcome.fingerprint.divergence,
        outcome.words
    )
}

#[must_use]
pub(crate) fn render_semantic_summary(summary: &SemanticSummary) -> String {
    format!(
        "fuzz semantic {}: kinds={} witnesses={} findings={} refusals={}\n",
        summary.interpreter, summary.kinds, summary.witnesses, summary.findings, summary.refusals
    )
}

#[must_use]
pub(crate) fn render_raw_progress(processed: u64, domain: u64) -> String {
    format!("fuzz raw: {processed} of {domain} words")
}

#[must_use]
pub(crate) fn render_raw_summary(summary: &RawSummary) -> String {
    let destination = summary
        .output
        .as_ref()
        .map_or_else(String::new, |path| format!(" -> {}", path.display()));
    format!(
        "fuzz raw: {:?} {} of {} words; accepted={} refused={} panics={}{destination}\n",
        summary.status,
        summary.processed,
        summary.domain,
        summary.accepted,
        summary.refused,
        summary.panics,
    )
}
