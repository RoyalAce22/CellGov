//! Runtime type definitions: step result, step error, mode, and
//! pluggable factory aliases. Extracted from `runtime.rs` to keep
//! the facade focused on orchestration logic.

use crate::registry::RegisteredUnit;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::ExecutionStepResult;
use cellgov_lv2::{PpuThreadInitState, SpuInitState};
use cellgov_time::{Budget, Epoch, GuestTicks};

/// One pass of the runtime pipeline as observed from outside.
///
/// Returned by [`crate::Runtime::step`] on success. Carries the selected
/// unit, the unit's step result (with emitted effects in stable
/// order), and the runtime's time/epoch values *after* the step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStep {
    /// Which unit was selected and run.
    pub unit: UnitId,
    /// What the unit returned from `run_until_yield`.
    pub result: ExecutionStepResult,
    /// Effects emitted during this step, in the order the unit emitted
    /// them. The runtime never reorders.
    pub effects: Vec<Effect>,
    /// Guest time after this step's consumed budget was applied.
    pub time_after: GuestTicks,
    /// Epoch after this step. The epoch advances only at commit
    /// boundaries, which the commit pipeline owns; `step()` does not
    /// advance the epoch and the value is unchanged from before the
    /// step.
    pub epoch_after: Epoch,
}

/// Why a [`crate::Runtime::step`] call could not produce a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepError {
    /// Registry is empty or every registered unit has
    /// `UnitStatus::Faulted` / `UnitStatus::Finished`. No future
    /// transition will wake anything -- this is a terminal stall.
    /// Call sites typically treat this as "run complete, no more
    /// work."
    NoRunnableUnit,
    /// Registry is non-empty and at least one unit is in
    /// `UnitStatus::Blocked`, but `count_runnable()` is zero. This
    /// is a scheduler-side liveness probe, not a semantic
    /// deadlock proof -- under the current sync surface it
    /// typically means every unit is parked on a PPU thread
    /// join chain. Once richer sync primitives land it also
    /// covers units parked on mutexes, condition variables,
    /// event queues, or external signals (audio, vblank, RSX
    /// label) that have not yet arrived. The caller decides
    /// whether to inject a pending wake, advance time, or treat
    /// this as a stall.
    AllBlocked,
    /// The runtime has already executed `max_steps` steps. Further
    /// stepping is the deadlock detector firing; the caller must
    /// abort the run rather than retry.
    MaxStepsExceeded,
    /// The runtime tried to advance guest time past `u64::MAX`. This
    /// is a runtime invariant violation in any realistic scenario;
    /// surfaced as an error rather than a panic so tests can assert
    /// on it.
    TimeOverflow,
}

/// Factory that constructs an SPU execution unit from an init state.
/// The runtime calls this when `Lv2Dispatch::RegisterSpu` fires.
/// The factory receives the `UnitId` the registry allocated and the
/// `SpuInitState` the LV2 host produced; it returns a boxed unit
/// ready to run.
pub type SpuFactory = Box<dyn Fn(UnitId, SpuInitState) -> Box<dyn RegisteredUnit>>;

/// Factory that constructs a child PPU execution unit from an
/// init state. The CLI installs one at boot via
/// [`crate::Runtime::set_ppu_factory`]; the runtime invokes it from
/// [`cellgov_lv2::Lv2Dispatch::PpuThreadCreate`] handling in
/// `commit_step`.
pub type PpuFactory = Box<dyn Fn(UnitId, PpuThreadInitState) -> Box<dyn RegisteredUnit>>;

/// Controls the runtime's overhead profile: which trace records are
/// emitted and whether state-hash checkpoints are computed at commit
/// boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    /// Trace off, hash checkpoints off. Minimal per-step bookkeeping.
    FaultDriven,
    /// Trace on (commits + block/wake only), hash checkpoints on.
    /// For microtest replay and oracle comparison.
    DeterminismCheck,
    /// All trace records, all hash checkpoints.
    /// For exploration and debugging.
    FullTrace,
}

/// Default per-step budget for a given mode. `FullTrace` returns 1
/// because per-step `PpuStateHash` records require single-instruction
/// yields to attribute hashes to specific PCs; the other modes
/// return the throughput batch size (256), where basic-block batching
/// and store forwarding give the foundational ~5x speedup over
/// Budget=1. Callers may override via [`crate::Runtime::set_budget`].
pub fn default_budget_for_mode(mode: RuntimeMode) -> Budget {
    match mode {
        RuntimeMode::FullTrace => Budget::new(1),
        RuntimeMode::FaultDriven | RuntimeMode::DeterminismCheck => Budget::new(256),
    }
}
