//! Repeated-trial evaluation of one engine and generator at equal budgets.
//!
//! One number from one seed says nothing about a generator. The outcome of a
//! randomized campaign is a distribution, and an evaluation samples it with
//! repeated trials at the same budget. [Klees2018 p:2127 s:4 Statistically Sound Comparisons]
//! A plan lists every seed before a trial runs. The results record every
//! trial the plan named. A comparison ranks two results only at equal
//! budgets, with a rank-based statistic over the full distributions.
//! [Klees2018 p:2128 s:4 Statistically Sound Comparisons]
//!
//! The library runs trials and folds them into records; the caller owns the
//! clock, the file system, and the host description it stores beside them.

mod comparison;
mod plan;
mod results;
mod statistics;
mod trial;

pub use comparison::{
    comparable, compare, Comparison, ComparisonError, ComparisonVerdict, MetricComparison,
    MetricVerdict, ResultsSide,
};
pub use plan::{EvaluationPlan, EvaluationPlanError, TrialBudget, MIN_TRIALS};
pub use results::{
    summarize, Direction, EvaluationEnvironment, EvaluationResults, EvaluationSummary, Metric,
    ResultsError, CRATE_VERSION, EVALUATION_SCHEMA_VERSION,
};
pub use statistics::{Distribution, Magnitude, Superiority};
pub use trial::{run_trial, run_trial_with, ReductionCost, TrialCounts, TrialOutcome, TrialRecord};

#[cfg(test)]
#[path = "../tests/evaluation_tests.rs"]
mod tests;
