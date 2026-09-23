//! A rank-based comparison of two evaluations at equal budgets.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use super::plan::EvaluationPlan;
use super::results::{Direction, EvaluationResults, Metric, ResultsError};
use super::statistics::{Distribution, Magnitude, Superiority};
use super::trial::TrialOutcome;

/// What one metric says about the candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricVerdict {
    /// The candidate's distribution sits better by at least a medium effect.
    Improved,
    /// The candidate's distribution sits worse by at least a medium effect.
    Regressed,
    /// The distributions overlap too much to rank.
    Indistinguishable,
}

/// One metric's distributions on both sides and their ranking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricComparison {
    /// Metric compared.
    pub metric: Metric,
    /// Which way an improvement points.
    pub direction: Direction,
    /// Baseline distribution.
    pub baseline: Distribution,
    /// Candidate distribution.
    pub candidate: Distribution,
    /// Probability that a candidate sample exceeds a baseline sample.
    pub superiority: Superiority,
    /// Effect size band.
    pub magnitude: Magnitude,
    /// How this metric ranks the candidate.
    pub verdict: MetricVerdict,
}

/// What the comparison says about the candidate as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonVerdict {
    /// A validity or coverage metric regressed.
    Regressed,
    /// No validity or coverage metric regressed, and at least one improved.
    Improved,
    /// No validity or coverage metric regressed or improved.
    Indistinguishable,
}

/// Two evaluations of the same engine at the same budget, metric by metric.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Comparison {
    /// Trials on each side.
    pub trials: u64,
    /// Budget shared by every trial on both sides.
    pub cases_per_trial: u64,
    /// Every metric both sides sampled, in report order.
    pub metrics: Vec<MetricComparison>,
}

impl Comparison {
    /// Validity and coverage metrics the candidate regressed on.
    #[must_use]
    pub fn regressions(&self) -> Vec<Metric> {
        self.metrics
            .iter()
            .filter(|metric| {
                metric.metric.guards_coverage() && metric.verdict == MetricVerdict::Regressed
            })
            .map(|metric| metric.metric)
            .collect()
    }

    /// The whole-comparison verdict.
    #[must_use]
    pub fn verdict(&self) -> ComparisonVerdict {
        if !self.regressions().is_empty() {
            ComparisonVerdict::Regressed
        } else if self.metrics.iter().any(|metric| {
            metric.metric.guards_coverage() && metric.verdict == MetricVerdict::Improved
        }) {
            ComparisonVerdict::Improved
        } else {
            ComparisonVerdict::Indistinguishable
        }
    }
}

/// Which stored result a refusal names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultsSide {
    /// The baseline.
    Baseline,
    /// The candidate.
    Candidate,
}

impl std::fmt::Display for ResultsSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Baseline => "baseline",
            Self::Candidate => "candidate",
        })
    }
}

/// Two results a comparison cannot rank against each other.
#[derive(Debug, thiserror::Error)]
pub enum ComparisonError {
    /// One side is not a complete evaluation.
    #[error("evaluation {side}: {source}")]
    Invalid {
        /// The refused side.
        side: ResultsSide,
        /// Why that side is not complete.
        #[source]
        source: ResultsError,
    },
    /// The sides evaluate different engines.
    #[error("evaluation baseline ran {baseline:?} but the candidate ran {candidate:?}")]
    DifferentTarget {
        /// Baseline engine.
        baseline: crate::FuzzTarget,
        /// Candidate engine.
        candidate: crate::FuzzTarget,
    },
    /// The sides ran different budgets.
    #[error(
        "evaluation baseline budgets {baseline} cases per trial but the candidate {candidate}"
    )]
    DifferentBudget {
        /// Baseline cases per trial.
        baseline: u64,
        /// Candidate cases per trial.
        candidate: u64,
    },
    /// The sides ran sequence cases of different lengths.
    #[error(
        "evaluation baseline runs {baseline} words per sequence case but the candidate {candidate}"
    )]
    DifferentSequenceWords {
        /// Baseline words per sequence case.
        baseline: u32,
        /// Candidate words per sequence case.
        candidate: u32,
    },
    /// The sides ran different trial counts.
    #[error("evaluation baseline ran {baseline} trials but the candidate {candidate}")]
    DifferentTrialCount {
        /// Baseline trials.
        baseline: usize,
        /// Candidate trials.
        candidate: usize,
    },
    /// One side holds a trial the harness did not finish.
    #[error("evaluation {side}: trial seed {seed} failed inside the harness")]
    HarnessFailed {
        /// The refused side.
        side: ResultsSide,
        /// Seed of the unfinished trial.
        seed: u64,
    },
}

/// Checks that a comparison can rank the results of two plans.
///
/// The plans agree on:
///
/// - the engine
/// - the budget
/// - the words per sequence case, on a sequence engine
/// - the trial count
///
/// A ranking between their results
/// then ranks the generators and nothing else. [Klees2018 p:2126 s:Overview]
/// A caller with a baseline in hand calls this before it runs the
/// candidate's trials. A plan the baseline can never rank then costs no trial.
///
/// # Errors
///
/// Names the first parameter the plans disagree on.
pub fn comparable(
    baseline: &EvaluationPlan,
    candidate: &EvaluationPlan,
) -> Result<(), ComparisonError> {
    if baseline.target != candidate.target {
        return Err(ComparisonError::DifferentTarget {
            baseline: baseline.target,
            candidate: candidate.target,
        });
    }
    if baseline.budget != candidate.budget {
        return Err(ComparisonError::DifferentBudget {
            baseline: baseline.budget.cases,
            candidate: candidate.budget.cases,
        });
    }
    // Words per case bound the instructions a sequence trial can execute. Two
    // lengths are two budgets, and every coverage metric would rank the
    // length. [Klees2018 p:2126 s:Overview]
    if matches!(
        baseline.target,
        crate::FuzzTarget::PpuSequence | crate::FuzzTarget::SpuSequence
    ) && baseline.sequence_words != candidate.sequence_words
    {
        return Err(ComparisonError::DifferentSequenceWords {
            baseline: baseline.sequence_words,
            candidate: candidate.sequence_words,
        });
    }
    if baseline.seeds.len() != candidate.seeds.len() {
        return Err(ComparisonError::DifferentTrialCount {
            baseline: baseline.seeds.len(),
            candidate: candidate.seeds.len(),
        });
    }
    Ok(())
}

/// Ranks `candidate` against `baseline` on every metric both sampled.
///
/// The plans pass [`comparable`] first: a comparison at unequal budgets
/// ranks the budget. [Klees2018 p:2126 s:Overview] A trial the harness did
/// not finish measured nothing, so `compare` refuses a side that holds one.
///
/// # Errors
///
/// Refuses an incomplete result, a side with a harness-failed trial, or
/// unequal engines, budgets, sequence lengths, or trial counts.
pub fn compare(
    baseline: &EvaluationResults,
    candidate: &EvaluationResults,
) -> Result<Comparison, ComparisonError> {
    finished(baseline, ResultsSide::Baseline)?;
    finished(candidate, ResultsSide::Candidate)?;
    comparable(&baseline.plan, &candidate.plan)?;
    let metrics = Metric::ALL
        .into_iter()
        .filter_map(|metric| {
            let left = baseline.summary.distributions.get(&metric)?;
            let right = candidate.summary.distributions.get(&metric)?;
            let superiority = Superiority::of(&right.samples, &left.samples)?;
            Some(MetricComparison {
                metric,
                direction: metric.direction(),
                baseline: left.clone(),
                candidate: right.clone(),
                superiority,
                magnitude: superiority.magnitude(),
                verdict: verdict(metric.direction(), superiority),
            })
        })
        .collect();
    Ok(Comparison {
        trials: baseline.trials.len() as u64,
        cases_per_trial: baseline.plan.budget.cases,
        metrics,
    })
}

/// Checks that `results` is complete and that the harness finished every trial.
fn finished(results: &EvaluationResults, side: ResultsSide) -> Result<(), ComparisonError> {
    results
        .validate()
        .map_err(|source| ComparisonError::Invalid { side, source })?;
    match results
        .trials
        .iter()
        .find(|trial| matches!(trial.outcome, TrialOutcome::HarnessFailure { .. }))
    {
        Some(trial) => Err(ComparisonError::HarnessFailed {
            side,
            seed: trial.seed,
        }),
        None => Ok(()),
    }
}

fn verdict(direction: Direction, superiority: Superiority) -> MetricVerdict {
    if superiority.magnitude() < Magnitude::Medium {
        return MetricVerdict::Indistinguishable;
    }
    let candidate_higher = superiority.favours() == Ordering::Greater;
    match (direction, candidate_higher) {
        (Direction::HigherIsBetter, true) | (Direction::LowerIsBetter, false) => {
            MetricVerdict::Improved
        }
        (Direction::HigherIsBetter, false) | (Direction::LowerIsBetter, true) => {
            MetricVerdict::Regressed
        }
    }
}
