//! `boot run`: drive one title to a terminal state and report it.

mod artifacts;
mod options;
mod report;
mod stages;

pub use options::{RunArtifacts, RunExecution, RunReporting};
pub(crate) use stages::configure_rsx_from_manifest;
pub use stages::{run_game, RunSummary};
