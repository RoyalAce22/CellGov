//! Runtime type definitions: step result, step error, mode, and
//! factory aliases.

use crate::registry::RegisteredUnit;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::ExecutionStepResult;
use cellgov_lv2::{PpuThreadInitState, SpuInitState};
use cellgov_time::{Budget, Epoch, GuestTicks};

/// One pass of the runtime pipeline, returned by
/// [`crate::Runtime::step`] on success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStep {
    /// The scheduled unit.
    pub unit: UnitId,
    /// Execution result from `run_until_yield`.
    pub result: ExecutionStepResult,
    /// Emission order preserved; the runtime never reorders.
    pub effects: Vec<Effect>,
    /// Guest time after the step's consumed budget was applied.
    pub time_after: GuestTicks,
    /// Epoch observed at step completion (unchanged within a step;
    /// advances only at commit boundaries).
    pub epoch_after: Epoch,
}

/// Why a [`crate::Runtime::step`] call could not produce a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StepError {
    /// Terminal stall: registry empty or every unit Faulted / Finished.
    #[error("no runnable unit")]
    NoRunnableUnit,
    /// At least one unit Blocked, none runnable. Liveness probe, not
    /// semantic deadlock proof; the caller decides whether to inject
    /// a pending wake, advance time, or treat it as a stall.
    #[error("all units blocked")]
    AllBlocked,
    /// Deadlock detector: `max_steps` reached. Callers must abort.
    #[error("max-steps cap exceeded")]
    MaxStepsExceeded,
    /// Consumed budget would push guest time past `u64::MAX`.
    #[error("guest time would overflow u64")]
    TimeOverflow,
    /// `Runtime::restore_into` ran without a subsequent
    /// `Runtime::set_scheduler`. Presence is mechanically checked;
    /// internal-state consistency with `snap` is the caller's
    /// obligation.
    #[error(
        "Runtime::step called between restore_into and set_scheduler; \
         install a fresh scheduler after every restore_into"
    )]
    SchedulerNotReinstalled,
}

/// Constructs an SPU unit when `Lv2Dispatch::RegisterSpu` fires.
pub type SpuFactory = Box<dyn Fn(UnitId, SpuInitState) -> Box<dyn RegisteredUnit>>;

/// Constructs a child PPU unit when `Lv2Dispatch::PpuThreadCreate` fires.
pub type PpuFactory = Box<dyn Fn(UnitId, PpuThreadInitState) -> Box<dyn RegisteredUnit>>;

/// Primary-thread seed a [`ProcessSpawnLoader`] reports after
/// installing a spawned image into its child space.
///
/// Architecture-specific ELF parsing stays with the loader's
/// implementor; the runtime consumes only these seeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpawnedProcessImage {
    /// Entry NIP (resolved OPD word, not the descriptor address).
    pub entry_code: u64,
    /// Entry r2/TOC (resolved OPD word).
    pub entry_toc: u64,
    /// Initial r1 for the primary thread.
    pub stack_top: u64,
    /// Initial LR; entered if the entry function returns.
    pub lr_sentinel: u64,
}

/// Failure surface of a [`ProcessSpawnLoader`].
///
/// Architecture-specific parse/load errors are rendered to text by
/// the implementor (this crate cannot name arch error types without
/// a backward DAG edge); memory errors keep their typed source.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProcessSpawnLoadError {
    /// The child image's headers did not parse.
    #[error("child image parse failed: {detail}")]
    ImageParse {
        /// Implementor-rendered parse failure.
        detail: String,
    },
    /// The child space cannot fit the image.
    #[error("child region sizing failed: {detail}")]
    RegionSize {
        /// Implementor-rendered sizing failure.
        detail: String,
    },
    /// Installing the child's backing region failed.
    #[error("child region install failed: {source}")]
    RegionInstall {
        /// Rejection from the guest-memory region table.
        #[source]
        source: cellgov_mem::MemError,
    },
    /// Seeding the child's exit stub failed.
    #[error("child exit stub write failed: {source}")]
    ExitStubWrite {
        /// Rejection from the guest-memory commit path.
        #[source]
        source: cellgov_mem::MemError,
    },
    /// Loading the image's segments failed.
    #[error("child image load failed: {detail}")]
    ImageLoad {
        /// Implementor-rendered load failure.
        detail: String,
    },
}

/// Installs a spawned SELF's image into a fresh child-space memory
/// when `Lv2Dispatch::ProcessSpawn` fires: region installs, segment
/// loads, and stack placement all belong to the loader. On `Err` the
/// runtime rolls the spawn back and fails the syscall.
pub type ProcessSpawnLoader = Box<
    dyn Fn(
        &[u8],
        &mut cellgov_mem::GuestMemory,
    ) -> Result<SpawnedProcessImage, ProcessSpawnLoadError>,
>;

/// Controls which trace records emit and whether per-instruction
/// state-hash fingerprints are captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    /// No trace records, no per-instruction hashes.
    FaultDriven,
    /// Commit + block/wake records + per-instruction hashes.
    DeterminismCheck,
    /// All trace records + per-instruction hashes.
    FullTrace,
}

/// All current modes return 256 (the throughput batch size).
pub fn default_budget_for_mode(mode: RuntimeMode) -> Budget {
    match mode {
        RuntimeMode::FullTrace | RuntimeMode::FaultDriven | RuntimeMode::DeterminismCheck => {
            Budget::new(256)
        }
    }
}
