//! `diff`-family subcommand handlers: scenario/manifest compare,
//! observation diff, streaming state-trace divergence, and per-step
//! register-level zoom.

mod dispatch;
mod manifest;
mod observations;
mod scenario;
mod trace;

pub(crate) use dispatch::run;
pub(crate) use observations::{load_observations_from_dir, run_compare_observations};
pub(super) use scenario::report_first_invariant_break;
pub(crate) use trace::{run_diverge, run_zoom};

#[cfg(test)]
#[path = "tests/compare_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/compare_disagreement_tests.rs"]
mod disagreement_tests;
