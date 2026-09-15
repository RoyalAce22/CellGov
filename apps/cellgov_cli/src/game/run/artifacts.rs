//! The files a finished `boot run` writes.

use cellgov_boot::manifest::TitleManifest;
use cellgov_boot::observation::{self, save_boot_observation};
use cellgov_compare::BootOutcome;
use cellgov_core::Runtime;
use cellgov_time::Budget;

use super::options::RunArtifacts;

/// Why the run refused an artifact.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// The `--save-observation` write failed.
    #[error("save-observation: {0}")]
    SaveObservation(#[source] observation::ObservationSaveError),
    /// The `--save-boot-summary` write failed.
    #[error("save-boot-summary: {0}")]
    SaveBootSummary(#[source] observation::ObservationSaveError),
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
/// [`RunError`] names the artifact that failed.
/// [`observation::ObservationSaveError`] says which failures leave a
/// partial file. This function writes the observation first, so a
/// refused observation leaves the summary unwritten. A
/// `--save-state-trace` write failure exits the process instead.
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
        std::fs::write(path, bytes).unwrap_or_else(|e| {
            crate::cli::exit::die(&format!(
                "save-state-trace: failed to write {} ({} bytes): {e}",
                path,
                bytes.len(),
            ))
        });
        eprintln!("save-state-trace: wrote {} bytes to {path}", bytes.len());
    }
    Ok(())
}
