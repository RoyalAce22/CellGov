//! Runtime type definitions: step result, step error, mode, and
//! factory aliases.

use crate::registry::RegisteredUnit;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::ExecutionStepResult;
use cellgov_lv2::{PpuThreadInitState, SpuInitState};
use cellgov_time::{Budget, Epoch, GuestTicks};

/// One pass of the runtime pipeline, returned by
/// [`crate::Runtime::step`] on success. `epoch_after` is unchanged from
/// before the step -- the epoch advances only at commit boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStep {
    /// Selected unit.
    pub unit: UnitId,
    /// Return value of `run_until_yield`.
    pub result: ExecutionStepResult,
    /// Emitted in the order the unit produced them; the runtime never
    /// reorders.
    pub effects: Vec<Effect>,
    /// Guest time after the step's consumed budget was applied.
    pub time_after: GuestTicks,
    /// Epoch value observed at step completion.
    pub epoch_after: Epoch,
}

/// Why a [`crate::Runtime::step`] call could not produce a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepError {
    /// Terminal stall: registry empty or every unit Faulted / Finished.
    NoRunnableUnit,
    /// At least one unit Blocked, none runnable. Liveness probe, not
    /// semantic deadlock proof; the caller decides whether to inject
    /// a pending wake, advance time, or treat it as a stall.
    AllBlocked,
    /// Deadlock detector: `max_steps` reached. Callers must abort.
    MaxStepsExceeded,
    /// Consumed budget would push guest time past `u64::MAX`.
    TimeOverflow,
}

/// Constructs an SPU unit when `Lv2Dispatch::RegisterSpu` fires.
pub type SpuFactory = Box<dyn Fn(UnitId, SpuInitState) -> Box<dyn RegisteredUnit>>;

/// Constructs a child PPU unit when `Lv2Dispatch::PpuThreadCreate` fires.
pub type PpuFactory = Box<dyn Fn(UnitId, PpuThreadInitState) -> Box<dyn RegisteredUnit>>;

/// Controls which trace records emit and whether state-hash
/// checkpoints compute at commit boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    /// Trace off, hash checkpoints off.
    FaultDriven,
    /// Commit + block/wake trace records, hash checkpoints on.
    DeterminismCheck,
    /// All trace records, all hash checkpoints.
    FullTrace,
}

/// `FullTrace` returns Budget=1 because `PpuStateHash` records require
/// single-instruction yields to attribute hashes to specific PCs; other
/// modes return the throughput batch size (256).
pub fn default_budget_for_mode(mode: RuntimeMode) -> Budget {
    match mode {
        RuntimeMode::FullTrace => Budget::new(1),
        RuntimeMode::FaultDriven | RuntimeMode::DeterminismCheck => Budget::new(256),
    }
}
