//! The three groups of options `boot run` takes.
//!
//! The split is by consumer: the boot library takes [`RunExecution`],
//! the run writes [`RunArtifacts`] to disk, and [`RunReporting`] goes to
//! the console.

use cellgov_boot::prepare::{DiagnosticOptions, ExecutionOptions, TitleOptions};

/// What the boot runs; [`cellgov_boot::prepare::prepare`] takes it
/// unchanged.
pub struct RunExecution<'a> {
    /// The title and the tree it resolves against.
    pub title: TitleOptions<'a>,
    /// How far the run goes and what it may change.
    pub limits: ExecutionOptions<'a>,
}

/// The files the run writes; a `None` path writes nothing.
pub struct RunArtifacts<'a> {
    /// `--save-observation`: the checkpoint observation JSON.
    pub observation: Option<&'a str>,
    /// Regions from a `--observation-manifest` the caller already
    /// parsed; `None` captures one region per PT_LOAD segment.
    pub observation_regions: Option<&'a [cellgov_compare::RegionDescriptor]>,
    /// `--save-boot-summary`: the summary JSON the anchor gate reads.
    pub boot_summary: Option<&'a str>,
    /// `--save-state-trace`: the raw trace stream.
    ///
    /// Set this together with [`ExecutionOptions::capture_state_trace`],
    /// which puts the per-step state hash in the stream and picks the
    /// run's mode. [`crate::game::run_game`] refuses the two in
    /// disagreement.
    pub state_trace: Option<&'a str>,
}

/// Where the run reports, and what it reports.
pub struct RunReporting<'a> {
    /// The debug taps and banner the boot library installs.
    pub boot: DiagnosticOptions<'a>,
    /// Report every step as it retires.
    pub trace: bool,
    /// Measure the startup stages and the step loop, and report both.
    pub profile: bool,
    /// Count the PPU states the run passes through and the state-hash
    /// collisions among them; see [`cellgov_boot::taps::StateHashCensus`].
    pub state_hash_census: bool,
    /// Where the boot reports its phases and retired steps.
    pub progress: &'a dyn crate::progress::ProgressSink,
    /// The step count the run should end at, when its cell's anchor
    /// recorded one; see [`crate::game::anchor_finish_line`].
    pub finish_line: Option<u64>,
}
