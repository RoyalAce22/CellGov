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

/// All current modes return 256 (the throughput batch size). The
/// exhaustive match (no `_` arm) forces a new `RuntimeMode` variant
/// to pick its budget explicitly.
pub fn default_budget_for_mode(mode: RuntimeMode) -> Budget {
    match mode {
        RuntimeMode::FullTrace | RuntimeMode::FaultDriven | RuntimeMode::DeterminismCheck => {
            Budget::new(256)
        }
    }
}
