//! One trial: a campaign at the plan's budget, folded into counts a
//! distribution can rank.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::plan::{EvaluationPlan, EvaluationPlanError};
use crate::reduce::{reduce_finding, ReductionRequest, ReductionTransform};
use crate::report::{Finding, FindingKind, FuzzReport, FuzzRun, RunOutcome};
use crate::FuzzConfig;

/// How a trial's campaign ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TrialOutcome {
    /// Every case completed without a finding.
    Clean,
    /// A semantic check found a disagreement.
    SemanticFinding,
    /// Target code panicked.
    TargetPanic,
    /// No case reached a check.
    NoEligibleCases,
    /// Only unsupported or undefined cases, and no finding.
    Inapplicable,
    /// The schedule stopped at a cancellation boundary.
    Cancelled,
    /// The harness failed; the counts cover the cases before it did.
    HarnessFailure {
        /// The engine's failure, rendered.
        message: String,
    },
}

impl From<&RunOutcome> for TrialOutcome {
    fn from(outcome: &RunOutcome) -> Self {
        match outcome {
            RunOutcome::CleanCompletion => Self::Clean,
            RunOutcome::SemanticFinding => Self::SemanticFinding,
            RunOutcome::TargetPanic => Self::TargetPanic,
            RunOutcome::Cancelled => Self::Cancelled,
            RunOutcome::NoEligibleCases => Self::NoEligibleCases,
            RunOutcome::UnsupportedCase
            | RunOutcome::UndefinedCase
            | RunOutcome::UnsupportedAndUndefinedCases => Self::Inapplicable,
            RunOutcome::HarnessFailure(error) => Self::HarnessFailure {
                message: error.to_string(),
            },
        }
    }
}

/// Counts one trial's campaign produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialCounts {
    /// Cases considered.
    pub attempted: u64,
    /// Decoded cases, or decoded instruction words for a sequence engine.
    pub decoded: u64,
    /// Cases that reached their check.
    pub eligible: u64,
    /// Cases the target refused as unmodeled.
    pub unsupported: u64,
    /// Cases the architecture leaves undefined.
    pub undefined: u64,
    /// Instructions executed over every assessed case.
    pub executed_steps: u64,
    /// Deepest single case.
    pub max_executed_depth: u64,
    /// Distinct instruction kinds reached.
    pub instruction_kinds: u64,
    /// Distinct effect classes observed.
    pub effect_classes: u64,
    /// Metamorphic partners executed over every relation.
    pub metamorphic_executions: u64,
    /// Cases the retention policy kept.
    pub retained: u64,
}

/// Cost and yield of reducing the trial's retained findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReductionCost {
    /// Retained findings the reducer ran on.
    pub findings: u64,
    /// Findings that shrank.
    pub reduced: u64,
    /// Findings already minimal.
    pub irreducible: u64,
    /// Findings the reducer refused or ran out of budget on.
    pub failed: u64,
    /// Candidate evaluations spent over the findings that finished. A refused
    /// finding adds none.
    pub evaluations: u64,
    /// Words the reducer started from, summed over the findings that finished:
    /// the case word for an instruction finding, every word for a sequence.
    pub words_before: u64,
    /// Case words after reduction, summed over the findings that finished.
    pub words_after: u64,
}

/// One trial's record: everything a distribution or a replay needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialRecord {
    /// Trial seed; with the plan it replays the campaign exactly.
    pub seed: u64,
    /// How the campaign ended.
    pub outcome: TrialOutcome,
    /// Campaign counts.
    pub counts: TrialCounts,
    /// Findings by kind, retained or not.
    pub findings: BTreeMap<String, u64>,
    /// Distinct fingerprints among the retained findings.
    pub unique_fingerprints: u64,
    /// Cases before the first retained finding, when there was one.
    pub first_finding_offset: Option<u64>,
    /// Reduction cost, when the plan requested reduction.
    pub reduction: Option<ReductionCost>,
    /// Host wall time, when the caller measured it.
    pub wall_ms: Option<u64>,
}

impl TrialRecord {
    /// Cases the trial could not check.
    #[must_use]
    pub fn invalid_cases(&self) -> u64 {
        self.counts
            .unsupported
            .saturating_add(self.counts.undefined)
    }

    /// Findings that fail a campaign; the inapplicability kinds classify a
    /// case and count as none.
    #[must_use]
    pub fn finding_total(&self) -> u64 {
        let unsupported = format!("{:?}", FindingKind::Unsupported);
        let undefined = format!("{:?}", FindingKind::Undefined);
        self.findings
            .iter()
            .filter(|(kind, _)| **kind != unsupported && **kind != undefined)
            .fold(0u64, |total, (_, count)| total.saturating_add(*count))
    }
}

/// Runs one trial of `plan` under `seed` on the plan's engine.
///
/// # Errors
///
/// Refuses a plan no evaluation can run.
pub fn run_trial(plan: &EvaluationPlan, seed: u64) -> Result<TrialRecord, EvaluationPlanError> {
    plan.validate()?;
    Ok(run_trial_with(plan, seed, |config| plan.target.run(config)))
}

/// Runs one trial through a caller-supplied engine and records it.
pub fn run_trial_with(
    plan: &EvaluationPlan,
    seed: u64,
    run: impl FnOnce(FuzzConfig) -> FuzzRun,
) -> TrialRecord {
    let config = plan.trial_config(seed);
    let run = run(config);
    let report = &run.report;
    let reduction = plan
        .reduction
        .map(|request| reduction_cost(config, &report.findings, request));
    TrialRecord {
        seed,
        outcome: TrialOutcome::from(&run.outcome),
        counts: counts(report),
        findings: report
            .finding_counts
            .iter()
            .map(|(kind, count)| (format!("{kind:?}"), *count))
            .collect(),
        unique_fingerprints: report
            .findings
            .iter()
            .map(|finding| finding.fingerprint)
            .collect::<BTreeSet<_>>()
            .len() as u64,
        first_finding_offset: report
            .findings
            .iter()
            .map(|finding| finding.replay.case_index)
            .min()
            .map(|index| index.saturating_sub(config.schedule.cases.first)),
        reduction,
        wall_ms: None,
    }
}

fn counts(report: &FuzzReport) -> TrialCounts {
    TrialCounts {
        attempted: report.cases,
        decoded: report.decoded,
        eligible: report.eligible_cases,
        unsupported: report.unsupported_cases,
        undefined: report.undefined_cases,
        executed_steps: report.executed_steps,
        max_executed_depth: report.max_executed_depth,
        instruction_kinds: report.instruction_kinds.len() as u64,
        effect_classes: report.effect_classes.len() as u64,
        metamorphic_executions: report
            .metamorphic_executions
            .values()
            .fold(0u64, |total, count| total.saturating_add(*count)),
        retained: report.retained_cases.entries().len() as u64,
    }
}

fn reduction_cost(
    config: FuzzConfig,
    findings: &[Finding],
    request: ReductionRequest,
) -> ReductionCost {
    let mut cost = ReductionCost {
        findings: findings.len() as u64,
        reduced: 0,
        irreducible: 0,
        failed: 0,
        evaluations: 0,
        words_before: 0,
        words_after: 0,
    };
    for finding in findings {
        match reduce_finding(config, finding, request) {
            Ok(report) => {
                let dropped = report
                    .applied
                    .iter()
                    .filter(|transform| matches!(transform, ReductionTransform::DropWord { .. }))
                    .count() as u64;
                let after = report.case_words.len() as u64;
                cost.evaluations = cost.evaluations.saturating_add(report.evaluations);
                cost.words_before = cost
                    .words_before
                    .saturating_add(after.saturating_add(dropped));
                cost.words_after = cost.words_after.saturating_add(after);
                if report.reduced() {
                    cost.reduced += 1;
                } else if report.fixpoint {
                    cost.irreducible += 1;
                } else {
                    cost.failed += 1;
                }
            }
            Err(_) => cost.failed += 1,
        }
    }
    cost
}
