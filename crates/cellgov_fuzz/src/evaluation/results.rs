//! The machine-readable record of one evaluation: plan, host, every trial,
//! and the distributions over them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::plan::{EvaluationPlan, EvaluationPlanError};
use super::statistics::Distribution;
use super::trial::{TrialOutcome, TrialRecord};

/// Schema version an evaluation result carries.
pub const EVALUATION_SCHEMA_VERSION: u32 = 1;

/// Version of this crate, for a caller that records the library that ran.
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The host that ran the trials, as the caller describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationEnvironment {
    /// Version of the fuzz library that ran, as [`CRATE_VERSION`] names it.
    pub crate_version: String,
    /// Host operating system.
    pub os: String,
    /// Host architecture.
    pub arch: String,
    /// Trials the caller ran at once.
    pub workers: u32,
}

/// Whether a larger sample is the better one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// More is better.
    HigherIsBetter,
    /// Less is better.
    LowerIsBetter,
}

/// One per-trial number an evaluation distributes and a comparison ranks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    /// Cases that reached their check.
    Eligible,
    /// Instructions executed.
    ExecutedSteps,
    /// Deepest case.
    MaxExecutedDepth,
    /// Distinct instruction kinds reached.
    InstructionKinds,
    /// Distinct effect classes observed.
    EffectClasses,
    /// Metamorphic partners executed.
    MetamorphicExecutions,
    /// Cases retained.
    Retained,
    /// Findings, the inapplicability kinds excluded.
    Findings,
    /// Distinct fingerprints among retained findings.
    UniqueFingerprints,
    /// Unsupported and undefined cases.
    InvalidCases,
    /// Cases before the first finding; only trials with a finding sample it.
    FirstFindingOffset,
    /// Candidate evaluations the reducer spent; only reducing trials sample it.
    ReductionEvaluations,
    /// Host wall time; only timed trials sample it.
    WallMs,
}

impl Metric {
    /// Every metric, in report order.
    pub const ALL: [Self; 13] = [
        Self::Eligible,
        Self::ExecutedSteps,
        Self::MaxExecutedDepth,
        Self::InstructionKinds,
        Self::EffectClasses,
        Self::MetamorphicExecutions,
        Self::Retained,
        Self::Findings,
        Self::UniqueFingerprints,
        Self::InvalidCases,
        Self::FirstFindingOffset,
        Self::ReductionEvaluations,
        Self::WallMs,
    ];

    /// Which way an improvement points.
    #[must_use]
    pub const fn direction(self) -> Direction {
        match self {
            Self::Eligible
            | Self::ExecutedSteps
            | Self::MaxExecutedDepth
            | Self::InstructionKinds
            | Self::EffectClasses
            | Self::MetamorphicExecutions
            | Self::Retained
            | Self::Findings
            | Self::UniqueFingerprints => Direction::HigherIsBetter,
            Self::InvalidCases
            | Self::FirstFindingOffset
            | Self::ReductionEvaluations
            | Self::WallMs => Direction::LowerIsBetter,
        }
    }

    /// Whether a regression on this metric is a regression of the whole comparison.
    ///
    /// Eligible cases carry validity: a generator whose cases the target
    /// refuses loses them here. The invalid-case count itself does not
    /// guard, because a generator that decodes nothing has none.
    #[must_use]
    pub const fn guards_coverage(self) -> bool {
        matches!(
            self,
            Self::Eligible
                | Self::ExecutedSteps
                | Self::InstructionKinds
                | Self::EffectClasses
                | Self::MetamorphicExecutions
        )
    }

    /// This metric's sample from one trial, when the trial has one.
    #[must_use]
    pub fn sample(self, trial: &TrialRecord) -> Option<u64> {
        match self {
            Self::Eligible => Some(trial.counts.eligible),
            Self::ExecutedSteps => Some(trial.counts.executed_steps),
            Self::MaxExecutedDepth => Some(trial.counts.max_executed_depth),
            Self::InstructionKinds => Some(trial.counts.instruction_kinds),
            Self::EffectClasses => Some(trial.counts.effect_classes),
            Self::MetamorphicExecutions => Some(trial.counts.metamorphic_executions),
            Self::Retained => Some(trial.counts.retained),
            Self::Findings => Some(trial.finding_total()),
            Self::UniqueFingerprints => Some(trial.unique_fingerprints),
            Self::InvalidCases => Some(trial.invalid_cases()),
            Self::FirstFindingOffset => trial.first_finding_offset,
            Self::ReductionEvaluations => trial.reduction.map(|cost| cost.evaluations),
            Self::WallMs => trial.wall_ms,
        }
    }
}

/// Distributions over every trial of one evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationSummary {
    /// Trials the plan named and the results hold.
    pub trials: u64,
    /// Budget shared by every trial.
    pub cases_per_trial: u64,
    /// One distribution per metric at least one trial sampled.
    pub distributions: BTreeMap<Metric, Distribution>,
}

/// Distributions over `trials`; a metric no trial samples is absent.
#[must_use]
pub fn summarize(plan: &EvaluationPlan, trials: &[TrialRecord]) -> EvaluationSummary {
    EvaluationSummary {
        trials: trials.len() as u64,
        cases_per_trial: plan.budget.cases,
        distributions: Metric::ALL
            .into_iter()
            .filter_map(|metric| {
                Distribution::of(trials.iter().filter_map(|trial| metric.sample(trial)))
                    .map(|distribution| (metric, distribution))
            })
            .collect(),
    }
}

/// One evaluation, reproducible from its plan alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationResults {
    /// Result schema version.
    pub schema_version: u32,
    /// What ran.
    pub plan: EvaluationPlan,
    /// Where it ran.
    pub environment: EvaluationEnvironment,
    /// Every trial, in plan order.
    pub trials: Vec<TrialRecord>,
    /// Distributions over the trials.
    pub summary: EvaluationSummary,
}

impl EvaluationResults {
    /// Assembles and validates the results of every trial the plan named.
    ///
    /// # Errors
    ///
    /// Refuses trials that do not match the plan's seeds and budget.
    pub fn from_trials(
        plan: EvaluationPlan,
        environment: EvaluationEnvironment,
        trials: Vec<TrialRecord>,
    ) -> Result<Self, ResultsError> {
        let summary = summarize(&plan, &trials);
        let results = Self {
            schema_version: EVALUATION_SCHEMA_VERSION,
            plan,
            environment,
            trials,
            summary,
        };
        results.validate()?;
        Ok(results)
    }

    /// Parses and validates a stored result.
    ///
    /// # Errors
    ///
    /// Refuses malformed JSON, another schema, or trials that contradict the plan.
    pub fn parse_json(json: &str) -> Result<Self, ResultsError> {
        let results: Self = serde_json::from_str(json)?;
        results.validate()?;
        Ok(results)
    }

    /// Checks that the trials are exactly the plan's and the summary is theirs.
    ///
    /// A trial the harness did not finish keeps the cases it reached, so it may
    /// sit under the budget but never over it.
    ///
    /// # Errors
    ///
    /// Names the first missing, unexpected, or off-budget trial, or a summary
    /// that does not follow from the trials.
    pub fn validate(&self) -> Result<(), ResultsError> {
        if self.schema_version != EVALUATION_SCHEMA_VERSION {
            return Err(ResultsError::Version {
                found: self.schema_version,
                supported: EVALUATION_SCHEMA_VERSION,
            });
        }
        self.plan.validate()?;
        for (expected, trial) in self.plan.seeds.iter().zip(&self.trials) {
            if trial.seed != *expected {
                return Err(if self.plan.seeds.contains(&trial.seed) {
                    ResultsError::MissingTrial { seed: *expected }
                } else {
                    ResultsError::UnexpectedTrial { seed: trial.seed }
                });
            }
        }
        if let Some(seed) = self.plan.seeds.get(self.trials.len()) {
            return Err(ResultsError::MissingTrial { seed: *seed });
        }
        if let Some(trial) = self.trials.get(self.plan.seeds.len()) {
            return Err(ResultsError::UnexpectedTrial { seed: trial.seed });
        }
        let cases = self.plan.budget.cases;
        if let Some(trial) = self.trials.iter().find(|trial| match trial.outcome {
            TrialOutcome::HarnessFailure { .. } => trial.counts.attempted > cases,
            _ => trial.counts.attempted != cases,
        }) {
            return Err(ResultsError::UnequalBudget {
                seed: trial.seed,
                found: trial.counts.attempted,
                expected: self.plan.budget.cases,
            });
        }
        if self.summary != summarize(&self.plan, &self.trials) {
            return Err(ResultsError::SummaryMismatch);
        }
        Ok(())
    }

    /// Every trial's sample of `metric`.
    #[must_use]
    pub fn samples(&self, metric: Metric) -> Vec<u64> {
        self.trials
            .iter()
            .filter_map(|trial| metric.sample(trial))
            .collect()
    }
}

/// A stored result that does not describe one complete evaluation.
#[derive(Debug, thiserror::Error)]
pub enum ResultsError {
    /// The JSON did not parse.
    #[error("evaluation results JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    /// Another schema.
    #[error("evaluation results version {found} is unsupported; expected {supported}")]
    Version {
        /// Version read.
        found: u32,
        /// Version this build accepts.
        supported: u32,
    },
    /// The plan cannot run.
    #[error("evaluation results plan: {0}")]
    Plan(#[from] EvaluationPlanError),
    /// A planned seed has no trial.
    #[error("evaluation results omit the trial for seed {seed}")]
    MissingTrial {
        /// Seed without a trial.
        seed: u64,
    },
    /// A trial the plan did not name.
    #[error("evaluation results hold a trial for seed {seed} the plan does not list")]
    UnexpectedTrial {
        /// Unplanned seed.
        seed: u64,
    },
    /// A trial ran another budget.
    #[error("evaluation trial seed {seed} attempted {found} cases instead of {expected}")]
    UnequalBudget {
        /// Trial seed.
        seed: u64,
        /// Cases the trial attempted.
        found: u64,
        /// Cases the plan budgets.
        expected: u64,
    },
    /// The stored summary is not the trials' summary.
    #[error("evaluation results summary does not follow from its trials")]
    SummaryMismatch,
}
