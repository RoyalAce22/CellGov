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

/// Controls which trace records emit and whether per-instruction
/// state-hash fingerprints are captured.
///
/// Two orthogonal axes drive the modes:
///
/// 1. *Per-yield* records (`UnitScheduled`, `StepCompleted`,
///    `EffectEmitted`, commit-boundary state-hash checkpoints):
///    fire once per `run_until_yield` call. `FullTrace` enables
///    them all; `DeterminismCheck` enables the commit-boundary
///    subset; `FaultDriven` disables them.
/// 2. *Per-instruction* `PpuStateHash` fingerprints: fire once per
///    retired instruction. The runtime sets the
///    `ExecutionContext::trace_per_step` flag based on mode
///    (`FullTrace` and `DeterminismCheck` set it, `FaultDriven`
///    does not). The unit's per-step hash buffer accumulates
///    `(pc, hash)` per retirement at any budget size, so trace
///    fidelity is independent of slice granularity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    /// No trace records, no per-instruction hashes.
    FaultDriven,
    /// Commit + block/wake records + per-instruction hashes.
    DeterminismCheck,
    /// All trace records + per-instruction hashes.
    FullTrace,
}

/// All non-trivial modes return the throughput batch size (256).
/// `PpuStateHash` records carry their own retired-instruction PC and
/// the unit's per-step hash buffer accumulates one entry per retired
/// instruction independent of yield size, so the trace fidelity is
/// orthogonal to budget choice. The yield-boundary records
/// (`UnitScheduled`, `StepCompleted`, `EffectEmitted`) describe a
/// scheduler decision and a budget consumption; under FullTrace they
/// fire once per yield, not once per instruction.
pub fn default_budget_for_mode(mode: RuntimeMode) -> Budget {
    match mode {
        RuntimeMode::FullTrace | RuntimeMode::FaultDriven | RuntimeMode::DeterminismCheck => {
            Budget::new(256)
        }
    }
}
