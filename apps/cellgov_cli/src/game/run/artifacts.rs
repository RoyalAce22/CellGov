//! This module defines run refusals and writes files after a `boot run`.

use cellgov_boot::manifest::TitleManifest;
use cellgov_boot::observation::{self, save_boot_observation};
use cellgov_compare::BootOutcome;
use cellgov_core::Runtime;
use cellgov_spu::capture::LocalStoreCapture;
use cellgov_spu::state::SpuState;
use cellgov_spu::SpuExecutionUnit;
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
    /// The run holds no SPU unit to capture.
    #[error("save-spu-local-store: the run holds no SPU unit")]
    NoSpuUnit,
    /// The run holds several SPU units and the command named none.
    #[error(
        "save-spu-local-store: the run holds SPU units {}; pass --spu-unit to pick one",
        listed(units)
    )]
    SeveralSpuUnits {
        /// The ids of the run's SPU units.
        units: Vec<u64>,
    },
    /// The named unit is not one of the run's SPU units.
    #[error(
        "save-spu-local-store: unit {unit} is no SPU unit of the run; its SPU units are {}",
        listed(units)
    )]
    NotAnSpuUnit {
        /// The id the command named.
        unit: u64,
        /// The ids of the run's SPU units.
        units: Vec<u64>,
    },
    /// The `--save-spu-local-store` write failed.
    #[error("save-spu-local-store: failed to write {path}: {source}")]
    SaveSpuLocalStore {
        /// The output path the command received.
        path: String,
        /// The host write failure.
        #[source]
        source: std::io::Error,
    },
}

/// `units` as a comma-separated list, or `none`.
fn listed(units: &[u64]) -> String {
    if units.is_empty() {
        return "none".to_owned();
    }
    units
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The capture of the SPU unit `wanted` names among `spus`, or of the
/// only one, with its id.
pub(super) fn capture_spu_unit(
    spus: &[(u64, &SpuState)],
    wanted: Option<u64>,
) -> Result<(u64, LocalStoreCapture), RunError> {
    let units = || spus.iter().map(|(id, _)| *id).collect::<Vec<_>>();
    let (id, state) = match (wanted, spus) {
        (Some(unit), _) => {
            spus.iter()
                .find(|(id, _)| *id == unit)
                .ok_or_else(|| RunError::NotAnSpuUnit {
                    unit,
                    units: units(),
                })?
        }
        (None, []) => return Err(RunError::NoSpuUnit),
        (None, [only]) => only,
        (None, _) => return Err(RunError::SeveralSpuUnits { units: units() }),
    };
    Ok((*id, LocalStoreCapture::of(state)))
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
    if let Some(request) = &artifacts.spu_local_store {
        let spus: Vec<(u64, &SpuState)> = rt
            .registry()
            .iter()
            .filter_map(|(id, unit)| {
                let spu = unit.as_any().downcast_ref::<SpuExecutionUnit>()?;
                Some((id.raw(), spu.state()))
            })
            .collect();
        let (unit, capture) = capture_spu_unit(&spus, request.unit)?;
        let path = request.path;
        std::fs::write(path, capture.to_bytes()).map_err(|source| RunError::SaveSpuLocalStore {
            path: path.to_string(),
            source,
        })?;
        eprintln!(
            "save-spu-local-store: unit {unit} at pc 0x{:05x}: wrote {path}",
            capture.pc
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/artifacts_tests.rs"]
mod tests;
