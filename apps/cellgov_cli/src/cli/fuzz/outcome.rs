//! Typed outcome records for every `dev fuzz` result, the exit status each
//! one maps to, and the one place that renders their terminal text.
//!
//! Identity stays in the records: callers compare a fingerprint or a case
//! index as a value, and rendered text carries none.
//! [Chen2013 p:2 s:1 Introduction] A triage that filters failures by text
//! patterns over their output is the ad hoc one the fuzzer-taming work replaces.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use cellgov_fuzz::artifact::{ArtifactFingerprint, ArtifactReduction};
use cellgov_fuzz::decode_census::ClassCounts;
use cellgov_fuzz::evaluation::{Comparison, ComparisonVerdict, EvaluationSummary};
use cellgov_fuzz::raw_decode::{RawDecodeStatus, RawDecoder};
use cellgov_fuzz::regression::Regression;
use cellgov_fuzz::{FindingKind, FuzzTarget};

use crate::cli::exit::CommandExitCode;
use crate::cli::exit_codes;

/// Exit code: a deadline or `--cancel-after` ended the range before every
/// case ran, whether or not the campaign retained a finding.
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
/// Exit code: an evaluation regressed a validity or coverage metric against
/// its baseline.
pub(crate) const EXIT_REGRESSION: i32 = exit_codes::command_specific(16);
/// Exit code: a smoke campaign reached less than its coverage floor.
pub(crate) const EXIT_VACUOUS: i32 = exit_codes::command_specific(17);

pub(crate) use cellgov_fuzz::runner::{ArtifactRecord, CampaignOutcome, CampaignSummary};

/// How the terminal names one retained finding's artifact.
pub(crate) trait ArtifactRecordText {
    /// Exact command that replays this artifact's original case.
    fn replay_command(&self) -> String;

    /// One summary line for the finding, after `prefix`.
    fn render(&self, prefix: &str) -> String;
}

impl ArtifactRecordText for ArtifactRecord {
    fn replay_command(&self) -> String {
        format!("cellgov dev fuzz replay --artifact {}", self.path.display())
    }

    fn render(&self, prefix: &str) -> String {
        format!(
            "{prefix}version={} seed={} case={} kind={:?} check={} divergence={} reduction={} artifact={} {}\n",
            self.campaign_version,
            self.seed,
            self.case_index,
            self.finding_kind,
            self.fingerprint.check,
            self.fingerprint.divergence,
            describe_reduction(&self.reduction),
            if self.stored { "stored" } else { "not stored" },
            self.replay_command(),
        )
    }
}

fn describe_reduction(reduction: &ArtifactReduction) -> String {
    match reduction {
        ArtifactReduction::NotAttempted => "not attempted".to_owned(),
        ArtifactReduction::Reduced { words } => format!("reduced to {} words", words.len()),
        ArtifactReduction::Irreducible => "irreducible".to_owned(),
        ArtifactReduction::Failed { reason } => format!("failed ({reason})"),
    }
}

/// One smoke campaign's run, as the summary names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SmokeCampaignSummary {
    /// Campaign name from the smoke set.
    pub name: &'static str,
    /// Generator version the campaign ran under.
    pub campaign_version: u32,
    /// Master seed.
    pub seed: u64,
    /// Cases considered.
    pub cases: u64,
    /// Cases eligible for their check.
    pub eligible: u64,
    /// Distinct instruction kinds executed.
    pub instruction_kinds: u64,
    /// Findings the engine counted, retained or not.
    pub findings: u64,
    /// Findings the engine retained with their evidence.
    pub retained: u64,
    /// Retained findings a promoted regression covers.
    pub promoted: u64,
    /// Findings no promoted regression covers: retained ones with no entry,
    /// and every counted finding past the retention bound.
    pub unpromoted: u64,
    /// Whether the run reached the campaign's coverage floor.
    pub covered: bool,
}

/// Terminal state of the smoke set, in exit precedence order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SmokeOutcome {
    /// The command could not store a finding's artifact.
    EvidenceNotStored,
    /// An engine failed inside the harness.
    HarnessFailure,
    /// A retained finding's reduction failed.
    ReductionFailed,
    /// A retained finding no promoted regression covers.
    Unpromoted,
    /// A campaign reached less than its coverage floor.
    Vacuous,
    /// Every campaign reached its floor and a promoted regression covers
    /// every finding.
    Clean,
}

impl SmokeOutcome {
    #[must_use]
    pub fn classify(
        evidence_not_stored: bool,
        harness_failure: bool,
        reductions_failed: u64,
        unpromoted: u64,
        vacuous: bool,
    ) -> Self {
        if evidence_not_stored {
            Self::EvidenceNotStored
        } else if harness_failure {
            Self::HarnessFailure
        } else if reductions_failed > 0 {
            Self::ReductionFailed
        } else if unpromoted > 0 {
            Self::Unpromoted
        } else if vacuous {
            Self::Vacuous
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
            Self::Unpromoted => CommandExitCode::new(exit_codes::FAILED),
            Self::Vacuous => CommandExitCode::new(EXIT_VACUOUS),
            Self::Clean => CommandExitCode::SUCCESS,
        }
    }
}

/// Renders one smoke campaign's line.
#[must_use]
pub(crate) fn render_smoke_campaign(summary: &SmokeCampaignSummary) -> String {
    format!(
        "fuzz smoke: {} version={} seed={} cases={} eligible={} instruction_kinds={} findings={} retained={} promoted={} unpromoted={} coverage={}\n",
        summary.name,
        summary.campaign_version,
        summary.seed,
        summary.cases,
        summary.eligible,
        summary.instruction_kinds,
        summary.findings,
        summary.retained,
        summary.promoted,
        summary.unpromoted,
        if summary.covered { "reached" } else { "under floor" },
    )
}

/// Renders one smoke finding's line, with the regression that covers it.
#[must_use]
pub(crate) fn render_smoke_finding(
    campaign: &str,
    record: &ArtifactRecord,
    promoted: Option<&str>,
) -> String {
    record.render(&format!(
        "fuzz smoke: finding campaign={campaign} promoted={} ",
        promoted.unwrap_or("none")
    ))
}

/// Renders the smoke set's closing line.
#[must_use]
pub(crate) fn render_smoke_outcome(campaigns: usize, outcome: SmokeOutcome) -> String {
    format!("fuzz smoke: campaigns={campaigns} outcome={outcome:?}\n")
}

/// Renders one promotion: the entry and the finding it now witnesses.
#[must_use]
pub(crate) fn render_promotion(regression: &Regression) -> String {
    format!(
        "fuzz promote: {} status={:?} profile={:?} kind={} check={} divergence={} artifact={}\n",
        regression.entry.name,
        regression.entry.status,
        regression.entry.profile,
        regression.artifact.finding_kind,
        regression.artifact.fingerprint.check,
        regression.artifact.fingerprint.divergence,
        regression.path.display(),
    )
}

/// The exit status each campaign outcome maps to.
pub(crate) trait CampaignExit {
    /// The documented exit status for this outcome.
    fn exit_code(self) -> CommandExitCode;
}

impl CampaignExit for CampaignOutcome {
    fn exit_code(self) -> CommandExitCode {
        match self {
            Self::EvidenceNotStored => CommandExitCode::new(EXIT_EVIDENCE_NOT_STORED),
            Self::HarnessFailure => CommandExitCode::new(EXIT_HARNESS_FAILURE),
            Self::ReductionFailed => CommandExitCode::new(EXIT_REDUCTION_FAILED),
            Self::Cancelled => CommandExitCode::new(EXIT_CANCELLED),
            Self::NoEligibleCases => CommandExitCode::new(EXIT_NO_ELIGIBLE_CASES),
            Self::Findings | Self::Clean => CommandExitCode::SUCCESS,
        }
    }
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
    /// Reproduction is the assertion a replay makes, so a reproduced
    /// finding is the failed-operation status and a vanished one is
    /// [`EXIT_NOT_REPRODUCED`].
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

/// One decoder census or merge of census shards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CensusSummary {
    /// Command that produced the result.
    pub command: &'static str,
    /// Words in the covered interval.
    pub words: u64,
    /// Counts by class.
    pub classes: ClassCounts,
    /// Words under primary opcode 0 that decoded.
    pub primary_zero_decoded: u64,
    /// Words that broke a census property.
    pub findings: u64,
    /// Wall time the command took.
    pub elapsed_seconds: u64,
    /// Path that received the versioned result.
    pub output: Option<PathBuf>,
}

impl CensusSummary {
    /// The documented exit status for this census.
    #[must_use]
    pub const fn exit_code(&self) -> CommandExitCode {
        if self.findings > 0 {
            CommandExitCode::new(exit_codes::FAILED)
        } else {
            CommandExitCode::SUCCESS
        }
    }
}

/// One stored evaluation, as the summary names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvaluationOutcome {
    /// Engine every trial ran.
    pub target: FuzzTarget,
    /// Path the evaluation wrote its results to.
    pub output: PathBuf,
    /// Distributions over the trials.
    pub summary: EvaluationSummary,
}

/// The exit status a comparison maps to.
#[must_use]
pub(crate) const fn comparison_exit_code(verdict: ComparisonVerdict) -> CommandExitCode {
    match verdict {
        ComparisonVerdict::Regressed => CommandExitCode::new(EXIT_REGRESSION),
        ComparisonVerdict::Improved | ComparisonVerdict::Indistinguishable => {
            CommandExitCode::SUCCESS
        }
    }
}

#[must_use]
pub(crate) fn render_evaluation_progress(seed: u64, completed: u64, trials: u64) -> String {
    format!("fuzz evaluate: trial seed={seed} done; {completed} of {trials} trials")
}

/// Renders an evaluation: one line for the run, then one per sampled metric.
#[must_use]
pub(crate) fn render_evaluation_summary(outcome: &EvaluationOutcome) -> String {
    let mut text = format!(
        "fuzz evaluate: {:?} trials={} cases_per_trial={} output={}\n",
        outcome.target,
        outcome.summary.trials,
        outcome.summary.cases_per_trial,
        outcome.output.display(),
    );
    for (metric, distribution) in &outcome.summary.distributions {
        text.push_str(&format!(
            "fuzz evaluate: {metric:?} samples={} min={} q1={} median={} q3={} max={}\n",
            distribution.samples.len(),
            distribution.minimum,
            distribution.lower_quartile,
            distribution.median,
            distribution.upper_quartile,
            distribution.maximum,
        ));
    }
    text
}

/// Renders a comparison: one line for the verdict, then one per metric.
#[must_use]
pub(crate) fn render_comparison(comparison: &Comparison) -> String {
    let regressions = comparison.regressions();
    let mut text = format!(
        "fuzz compare: trials={} cases_per_trial={} verdict={:?} regressions={:?}\n",
        comparison.trials,
        comparison.cases_per_trial,
        comparison.verdict(),
        regressions,
    );
    for metric in &comparison.metrics {
        text.push_str(&format!(
            "fuzz compare: {:?} baseline_median={} candidate_median={} superiority={}/{} magnitude={:?} verdict={:?}\n",
            metric.metric,
            metric.baseline.median,
            metric.candidate.median,
            metric.superiority.favourable,
            metric.superiority.pairs,
            metric.magnitude,
            metric.verdict,
        ));
    }
    text
}

/// The retained artifacts that share one finding kind, check and
/// divergence class: one summary line, however many cases hit it.
struct FindingGroup<'a> {
    kind: FindingKind,
    check: &'a str,
    divergence: &'a str,
    /// In storage order.
    records: Vec<&'a ArtifactRecord>,
}

/// Instruction kinds a group line names before it counts the rest.
const LISTED_INSTRUCTION_KINDS: usize = 4;

impl FindingGroup<'_> {
    fn render(&self) -> String {
        let kinds: BTreeSet<&str> = self
            .records
            .iter()
            .filter_map(|record| record.fingerprint.instruction_kind.as_deref())
            .collect();
        let instructions = if kinds.is_empty() {
            "none".to_owned()
        } else {
            let mut listed = kinds
                .iter()
                .take(LISTED_INSTRUCTION_KINDS)
                .copied()
                .collect::<Vec<_>>()
                .join(",");
            let rest = kinds.len().saturating_sub(LISTED_INSTRUCTION_KINDS);
            if rest > 0 {
                listed.push_str(&format!(",+{rest} more"));
            }
            listed
        };
        let mut line = format!(
            "fuzz: findings={} kind={:?} check={} divergence={} instructions={instructions}",
            self.records.len(),
            self.kind,
            self.check,
            self.divergence,
        );
        let (mut reduced, mut irreducible, mut failed) = (0usize, 0usize, 0usize);
        for record in &self.records {
            match record.reduction {
                ArtifactReduction::Reduced { .. } => reduced += 1,
                ArtifactReduction::Irreducible => irreducible += 1,
                ArtifactReduction::Failed { .. } => failed += 1,
                ArtifactReduction::NotAttempted => {}
            }
        }
        if reduced + irreducible + failed > 0 {
            line.push_str(&format!(
                " reduced={reduced} irreducible={irreducible} reduction_failed={failed}"
            ));
        }
        line.push_str(&format!(" replay: {}\n", self.replay().replay_command()));
        line
    }

    /// The earliest record whose artifact reached its path, so the
    /// replay names a file that exists; with none stored, the earliest.
    fn replay(&self) -> &ArtifactRecord {
        self.records
            .iter()
            .copied()
            .find(|record| record.stored)
            .unwrap_or(self.records[0])
    }
}

/// Group `records` by finding, largest group first, ties in kind order.
fn finding_groups(records: &[ArtifactRecord]) -> Vec<FindingGroup<'_>> {
    let mut groups: Vec<FindingGroup<'_>> = Vec::new();
    for record in records {
        let check = record.fingerprint.check.as_str();
        let divergence = record.fingerprint.divergence.as_str();
        let same = |group: &&mut FindingGroup<'_>| {
            group.kind == record.finding_kind
                && group.check == check
                && group.divergence == divergence
        };
        match groups.iter_mut().find(same) {
            Some(group) => group.records.push(record),
            None => groups.push(FindingGroup {
                kind: record.finding_kind,
                check,
                divergence,
                records: vec![record],
            }),
        }
    }
    groups.sort_by(|a, b| {
        b.records
            .len()
            .cmp(&a.records.len())
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.check.cmp(b.check))
            .then_with(|| a.divergence.cmp(b.divergence))
    });
    groups
}

/// Renders a campaign's summary: one status line, one line per
/// distinct finding with its count, its instruction kinds and one
/// replay command, then the artifact tally with its directory.
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
    for group in finding_groups(&summary.artifacts) {
        text.push_str(&group.render());
    }
    if let Some(first) = summary.artifacts.first() {
        let stored = summary.artifacts.iter().filter(|a| a.stored).count();
        let dir = first.path.parent().unwrap_or_else(|| Path::new(""));
        text.push_str(&format!(
            "fuzz: artifacts={} stored={stored} not_stored={} dir={}\n",
            summary.artifacts.len(),
            summary.artifacts.len() - stored,
            dir.display(),
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
pub(crate) fn render_census_progress(processed: u64, domain: u64) -> String {
    format!("fuzz census: {processed} of {domain} words")
}

#[must_use]
pub(crate) fn render_census_summary(summary: &CensusSummary) -> String {
    let destination = summary
        .output
        .as_ref()
        .map_or_else(String::new, |path| format!(" -> {}", path.display()));
    let classes = summary.classes;
    format!(
        "{}: {} words in {}s; canonical={} reserved_bits={} alias={} round_trip_failures={} \
         arm_unimplemented={} arm_unlisted={} not_recognized={} panics={} \
         primary_zero_decoded={} findings={}{destination}\n",
        summary.command,
        summary.words,
        summary.elapsed_seconds,
        classes.canonical,
        classes.reserved_bits,
        classes.alias,
        classes.round_trip_failures,
        classes.arm_unimplemented,
        classes.arm_unlisted,
        classes.not_recognized,
        classes.panics,
        summary.primary_zero_decoded,
        summary.findings,
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
