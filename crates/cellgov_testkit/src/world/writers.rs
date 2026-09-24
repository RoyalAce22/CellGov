//! The fixtures that write guest memory: every step, once from the clock, or once after a sleep.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_time::{Budget, GuestTicks, InstructionCost};
use std::cell::Cell;

/// Emits one `SharedWriteIntent` per step against `range`; finishes
/// after `max` steps.
///
/// The payload is the step number byte-replicated, or the byte
/// [`WritingUnit::of_value`] fixes.
#[derive(Clone)]
pub struct WritingUnit {
    id: UnitId,
    steps: Cell<u64>,
    max: u64,
    range: ByteRange,
    /// Byte every write carries, or `None` to write the step number.
    value: Option<u8>,
}

impl WritingUnit {
    /// Construct a unit writing into `range` once per step, finishing
    /// after `max` steps.
    pub fn new(id: UnitId, max: u64, range: ByteRange) -> Self {
        Self {
            id,
            steps: Cell::new(0),
            max,
            range,
            value: None,
        }
    }

    /// Like [`WritingUnit::new`], with `value` as the payload of every
    /// write.
    pub fn of_value(id: UnitId, max: u64, range: ByteRange, value: u8) -> Self {
        Self {
            value: Some(value),
            ..Self::new(id, max, range)
        }
    }

    /// 4 bytes at address 0.
    pub fn at_zero(id: UnitId, max: u64) -> Self {
        Self::new(id, max, ByteRange::new(GuestAddr::new(0), 4).unwrap())
    }
}

impl ExecutionUnit for WritingUnit {
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
        let byte = self.value.unwrap_or(n as u8);
        let bytes = vec![byte; self.range.length() as usize];
        effects.push(Effect::shared_write(
            self.range,
            WritePayload::new(bytes),
            self.id,
            GuestTicks::ZERO,
        ));
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

/// Writes the guest tick count it read into `range`, once, then
/// finishes.
///
/// The stand-in for a guest that reads the time base and stores it.
#[derive(Clone)]
pub struct ClockWriter {
    id: UnitId,
    range: ByteRange,
    steps: Cell<u64>,
}

impl ClockWriter {
    /// Construct a unit that stores the tick count it read into `range`.
    ///
    /// # Panics
    ///
    /// Panics if `range` is not the eight bytes a tick count occupies.
    pub fn new(id: UnitId, range: ByteRange) -> Self {
        assert_eq!(range.length(), 8, "a clock reader stores a whole count");
        Self {
            id,
            range,
            steps: Cell::new(0),
        }
    }
}

impl ExecutionUnit for ClockWriter {
    type Snapshot = u64;
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 1 {
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
        self.steps.set(self.steps.get() + 1);
        let ticks = ctx.current_tick().raw().to_le_bytes();
        effects.push(Effect::ClockRead { source: self.id });
        effects.push(Effect::shared_write(
            self.range,
            WritePayload::new(ticks.to_vec()),
            self.id,
            GuestTicks::ZERO,
        ));
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
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

/// Parks on `sys_timer_usleep`, then writes `value` over `range` once
/// the deadline fires and finishes.
///
/// The guest clock is global, so other units' tick spend moves this
/// unit's wake against their steps, which makes the workload
/// schedule-sensitive.
#[derive(Clone)]
pub struct SleepingWriter {
    id: UnitId,
    usec: u64,
    range: ByteRange,
    value: u8,
    steps: Cell<u64>,
}

impl SleepingWriter {
    /// Construct a unit that sleeps `usec` microseconds, then writes
    /// `value` over `range`.
    pub fn new(id: UnitId, usec: u64, range: ByteRange, value: u8) -> Self {
        Self {
            id,
            usec,
            range,
            value,
            steps: Cell::new(0),
        }
    }
}

impl ExecutionUnit for SleepingWriter {
    type Snapshot = u64;
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 2 {
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
        if n == 1 {
            let mut args = [0u64; 9];
            args[0] = cellgov_ps3_abi::lv2::syscall::TIMER_USLEEP;
            args[1] = self.usec;
            return ExecutionStepResult {
                yield_reason: YieldReason::Syscall,
                consumed_cost: InstructionCost::new(budget.raw()),
                local_diagnostics: LocalDiagnostics::with_pc(0x1000),
                fault: None,
                syscall_args: Some(args),
            };
        }
        let bytes = vec![self.value; self.range.length() as usize];
        effects.push(Effect::shared_write(
            self.range,
            WritePayload::new(bytes),
            self.id,
            GuestTicks::ZERO,
        ));
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
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
