//! The counting and polling fixtures.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::ByteRange;
use cellgov_time::{Budget, InstructionCost};
use std::cell::Cell;

/// Consumes its full budget each step, emits one `TraceMarker`, finishes
/// after `max` steps.
#[derive(Clone)]
pub struct CountingUnit {
    id: UnitId,
    steps: Cell<u64>,
    max: u64,
    /// Ticks each step spends, or `None` to spend the whole budget.
    cost: Option<u64>,
}

impl CountingUnit {
    /// Construct a unit that finishes after `max` steps.
    pub fn new(id: UnitId, max: u64) -> Self {
        Self {
            id,
            steps: Cell::new(0),
            max,
            cost: None,
        }
    }

    /// Like [`CountingUnit::new`], spending `cost` ticks per step
    /// instead of the whole budget.
    ///
    /// Two units that differ here spend different guest time for the
    /// same number of steps, which is what moves a pending deadline
    /// against a schedule that reorders them.
    pub fn of_cost(id: UnitId, max: u64, cost: u64) -> Self {
        Self {
            cost: Some(cost),
            ..Self::new(id, max)
        }
    }

    /// Steps executed so far.
    pub fn steps_taken(&self) -> u64 {
        self.steps.get()
    }
}

impl ExecutionUnit for CountingUnit {
    type Snapshot = u64;
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.steps.get() >= self.max {
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
        let yield_reason = if n >= self.max {
            YieldReason::Finished
        } else {
            YieldReason::BudgetExhausted
        };
        effects.push(Effect::TraceMarker {
            marker: n as u32,
            source: self.id,
        });
        // The runtime adds any cost to the clock without holding it
        // against the budget, so a fixture asking for more than a step
        // was given advances guest time in a way no unit can.
        debug_assert!(
            self.cost.is_none_or(|c| c <= budget.raw()),
            "counting unit {:?} spends {:?} ticks against a budget of {}",
            self.id,
            self.cost,
            budget.raw(),
        );
        ExecutionStepResult {
            yield_reason,
            consumed_cost: InstructionCost::new(self.cost.unwrap_or(budget.raw())),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }
    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

/// Reads one byte per step and finishes once it reads a non-zero.
///
/// Another unit writes that byte, so the schedule decides how many
/// steps the poller retires. Every step emits a `SharedReadIntent` for
/// the byte, so the independence relation sees the pair.
#[derive(Clone)]
pub struct PollingUnit {
    id: UnitId,
    steps: Cell<u64>,
    max: u64,
    done: Cell<bool>,
    range: ByteRange,
}

impl PollingUnit {
    /// Construct a unit that polls the byte at `range` for at most
    /// `max` steps.
    ///
    /// # Panics
    ///
    /// Panics if `range` is not one byte.
    pub fn new(id: UnitId, max: u64, range: ByteRange) -> Self {
        assert_eq!(range.length(), 1, "a poller reads one byte");
        Self {
            id,
            steps: Cell::new(0),
            max,
            done: Cell::new(false),
            range,
        }
    }
}

impl ExecutionUnit for PollingUnit {
    type Snapshot = (u64, bool);
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.done.get() || self.steps.get() >= self.max {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }
    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        // The commit pipeline stages nothing for `SharedReadIntent`, so
        // an unreadable range gives no refusal of its own.
        let bytes = ctx
            .memory()
            .read_checked(self.range)
            .expect("the polled range must be readable");
        let seen = *bytes.first().expect("a poller reads one byte");
        if seen != 0 {
            self.done.set(true);
        }
        effects.push(Effect::SharedReadIntent {
            range: self.range,
            source: self.id,
        });
        let yield_reason = if self.done.get() || n >= self.max {
            YieldReason::Finished
        } else {
            YieldReason::BudgetExhausted
        };
        ExecutionStepResult {
            yield_reason,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }
    fn snapshot(&self) -> (u64, bool) {
        (self.steps.get(), self.done.get())
    }
}
