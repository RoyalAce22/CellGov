//! `boot run`: drive one title to a terminal state and report it.

mod artifacts;
mod options;
mod report;
mod stages;
mod unmodelled;

pub use options::{RunArtifacts, RunExecution, RunReporting};
pub use stages::{run_game, RunSummary};
