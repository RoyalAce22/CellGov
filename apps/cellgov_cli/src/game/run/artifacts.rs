//! This module defines run refusals and writes files after a `boot run`.

use cellgov_boot::manifest::TitleManifest;
use cellgov_boot::observation::{self, save_boot_observation};
use cellgov_compare::BootOutcome;
use cellgov_core::Runtime;
use cellgov_time::Budget;

use super::options::RunArtifacts;

/// Preserves the refusal category for the CLI status contract.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// A shared command input or environment setting contains an invalid value.
    #[error("{0}")]
    Command(#[from] crate::cli::exit::CommandError),
    /// The state-trace path and capture mode disagree.
    #[error("{0}")]
    StateTraceConfiguration(String),
    /// A debug watch from the environment cannot start.
    #[error("{0}")]
    DebugTaps(String),
    /// Boot preparation or the diagnostic step loop refused the run.
    #[error("{0}")]
    Boot(#[from] cellgov_boot::BootError),
    /// The `--save-observation` write failed.
    #[error("save-observation: {0}")]
    SaveObservation(#[source] observation::ObservationSaveError),
    /// The `--save-boot-summary` write failed.
    #[error("save-boot-summary: {0}")]
    SaveBootSummary(#[source] observation::ObservationSaveError),
    /// The `--save-state-trace` write failed.
    #[error("save-state-trace: failed to write {path} ({bytes} bytes): {source}")]
    SaveStateTrace {
        /// The output path the command received.
        path: String,
        /// The trace size the command tried to write.
        bytes: usize,
        /// The host write failure.
        #[source]
        source: std::io::Error,
    },
}

impl RunError {
    /// `boot run` assigns observation and summary refusals to status 14.
    pub fn is_report_artifact_failure(&self) -> bool {
        matches!(self, Self::SaveObservation(_) | Self::SaveBootSummary(_))
    }
}

/// What the finished run contributes to every artifact it writes.
pub(super) struct RunFacts<'a> {
    pub title: &'a TitleManifest,
    /// The identity triple every artifact embeds.
    pub identity: &'a cellgov_compare::RunIdentity,
    /// The plaintext title ELF, whose PT_LOAD segments name the default
    /// observation regions.
    pub elf_data: &'a [u8],
    /// The terminal state the step loop reached.
    pub outcome: BootOutcome,
    /// Retired steps at that state.
    pub steps: usize,
    /// Retired instructions one `step()` granted.
    pub step_budget: Budget,
}

/// Write every artifact the caller asked for.
///
/// # Errors
///
/// Returns an error if an artifact write fails.
/// [`observation::ObservationSaveError`] says which failures leave a
/// partial file. This function writes the observation first, so a
/// refused observation leaves the summary unwritten.
pub(super) fn save_artifacts(
    rt: &mut Runtime,
    artifacts: &RunArtifacts<'_>,
    facts: &RunFacts<'_>,
    sink: &dyn cellgov_boot::BootSink,
) -> Result<(), RunError> {
    if let Some(path) = artifacts.observation {
        // Every space, so a checkpoint manifest can name a spawned
        // child's memory; a `GuestMemory` clone is a refcount bump.
        let final_spaces: cellgov_compare::SpaceSnapshots = rt
            .address_spaces()
            .map(|(id, mem)| (id, mem.clone()))
            .collect();
        save_boot_observation(observation::ObservationInputs {
            path,
            elf_data: facts.elf_data,
            final_spaces: &final_spaces,
            outcome: facts.outcome,
            steps: facts.steps,
            manifest_regions: artifacts.observation_regions,
            tty_log: &rt.lv2_host().observability().tty_log,
            identity: facts.identity,
            sink,
        })
        .map_err(RunError::SaveObservation)?;
    }
    if let Some(path) = artifacts.boot_summary {
        let host_invariant_breaks = rt.lv2_host().observability().invariant_break_count as u64;
        observation::save_boot_summary_json(observation::BootSummaryInputs {
            path,
            title: facts.title,
            outcome: facts.outcome,
            steps: facts.steps,
            step_budget: facts.step_budget,
            host_invariant_breaks,
            identity: facts.identity.clone(),
            sink,
        })
        .map_err(RunError::SaveBootSummary)?;
    }
    if let Some(path) = artifacts.state_trace {
        let bytes = rt.trace().bytes();
        std::fs::write(path, bytes).map_err(|source| RunError::SaveStateTrace {
            path: path.to_string(),
            bytes: bytes.len(),
            source,
        })?;
        eprintln!("save-state-trace: wrote {} bytes to {path}", bytes.len());
    }
    Ok(())
}
