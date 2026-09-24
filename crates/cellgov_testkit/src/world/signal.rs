//! The signal-notification fixture.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_sync::SignalId;
use cellgov_time::{Budget, InstructionCost};
use std::cell::Cell;

/// Emits one [`Effect::SignalUpdate`] per step into `target`, OR-ing in
/// `1 << (step - 1)`; finishes after `bit_count` steps leaving
/// `(1 << bit_count) - 1` in the register.
#[derive(Clone)]
pub struct SignalEmitter {
    id: UnitId,
    target: SignalId,
    steps: Cell<u64>,
    bit_count: u64,
}

impl SignalEmitter {
    /// Construct an emitter performing `bit_count` OR-merges into `target`.
    ///
    /// # Panics
    ///
    /// Panics if `bit_count > 32` (the signal register is `u32`).
    pub fn new(id: UnitId, target: SignalId, bit_count: u64) -> Self {
        assert!(
            bit_count <= 32,
            "SignalEmitter bit_count must be <= 32 (signal register is u32), got {bit_count}"
        );
        Self {
            id,
            target,
            steps: Cell::new(0),
            bit_count,
        }
    }
}

impl ExecutionUnit for SignalEmitter {
    type Snapshot = u64;
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.steps.get() >= self.bit_count {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }
    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        let yield_reason = if n >= self.bit_count {
            YieldReason::Finished
        } else {
            YieldReason::WaitingSync
        };
        let value = 1u32 << (n - 1) as u32;
        effects.push(Effect::SignalUpdate {
            signal: self.target,
            value,
            source: self.id,
        });
        ExecutionStepResult {
            yield_reason,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }
    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}
