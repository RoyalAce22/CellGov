//! Reusable fake [`ExecutionUnit`] implementations composed by scenario
//! fixtures.
//!
//! The fakes probe runtime wiring directly, independent of any real
//! architectural interpreter.

use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::{Effect, MailboxMessage, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_sync::{MailboxId, SignalId};
use cellgov_time::{Budget, GuestTicks, InstructionCost};
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
/// steps the poller retires.
///
/// Every step emits a `SharedReadIntent` for the byte, so the
/// independence relation sees the pair.
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
        // an unreadable range gives no refusal of its own. The expect
        // names the failure instead of leaving the poller to spin to
        // `max`.
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

/// Parks on `sys_timer_usleep`, then writes `value` over `range` once
/// the deadline fires and finishes.
///
/// The guest clock is global, so every other unit's tick spend moves
/// this unit's wake against their steps. That is what makes a workload
/// holding one of these schedule-sensitive.
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

/// Emits one [`Effect::MailboxSend`] per step into `target` with message
/// words `1..=max`; finishes after `max` steps.
#[derive(Clone)]
pub struct MailboxProducer {
    id: UnitId,
    target: MailboxId,
    steps: Cell<u64>,
    max: u64,
}

impl MailboxProducer {
    /// Construct a producer sending `max` messages into `target`.
    pub fn new(id: UnitId, target: MailboxId, max: u64) -> Self {
        Self {
            id,
            target,
            steps: Cell::new(0),
            max,
        }
    }
}

impl ExecutionUnit for MailboxProducer {
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
            YieldReason::MailboxAccess
        };
        effects.push(Effect::MailboxSend {
            mailbox: self.target,
            message: MailboxMessage::new(n as u32),
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

/// Two-stage DMA block/unblock probe: seed source, submit Put, block;
/// then on wake emit a `TraceMarker` and finish.
#[derive(Clone)]
pub struct DmaSubmitter {
    id: UnitId,
    source: ByteRange,
    destination: ByteRange,
    seed_bytes: Vec<u8>,
    phase: Cell<u8>,
}

impl DmaSubmitter {
    /// Construct a submitter writing `seed_bytes` to `source` then
    /// enqueuing a DMA Put from `source` to `destination`.
    ///
    /// # Panics
    ///
    /// Panics if `seed_bytes.len() != source.length()`.
    pub fn new(id: UnitId, source: ByteRange, destination: ByteRange, seed_bytes: Vec<u8>) -> Self {
        assert_eq!(
            seed_bytes.len() as u64,
            source.length(),
            "seed_bytes length must match source range"
        );
        Self {
            id,
            source,
            destination,
            seed_bytes,
            phase: Cell::new(0),
        }
    }
}

impl ExecutionUnit for DmaSubmitter {
    type Snapshot = u8;
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.phase.get() >= 2 {
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
        let p = self.phase.get();
        self.phase.set(p + 1);
        match p {
            0 => {
                let req =
                    DmaRequest::new(DmaDirection::Put, self.source, self.destination, self.id)
                        .expect("source and destination lengths match");
                effects.push(Effect::shared_write(
                    self.source,
                    WritePayload::new(self.seed_bytes.clone()),
                    self.id,
                    GuestTicks::ZERO,
                ));
                effects.push(Effect::DmaEnqueue {
                    request: req,
                    payload: None,
                });
                effects.push(Effect::WaitOnEvent {
                    target: cellgov_effects::WaitTarget::Barrier(cellgov_sync::BarrierId::new(0)),
                    source: self.id,
                });
                ExecutionStepResult {
                    yield_reason: YieldReason::DmaSubmitted,
                    consumed_cost: InstructionCost::new(budget.raw()),
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                    syscall_args: None,
                }
            }
            _ => {
                effects.push(Effect::TraceMarker {
                    marker: 0xd0d0,
                    source: self.id,
                });
                ExecutionStepResult {
                    yield_reason: YieldReason::Finished,
                    consumed_cost: InstructionCost::new(budget.raw()),
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                    syscall_args: None,
                }
            }
        }
    }
    fn snapshot(&self) -> u8 {
        self.phase.get()
    }
}

/// Three-stage PPU-like sender: send command + wake responder + wait;
/// receive attempt; consume response and emit a `TraceMarker`.
///
/// Explicit `WakeUnit` is required: the commit pipeline does not auto-wake
/// on message delivery.
#[derive(Clone)]
pub struct MailboxSender {
    id: UnitId,
    responder: UnitId,
    cmd_mailbox: MailboxId,
    resp_mailbox: MailboxId,
    command: u32,
    phase: Cell<u8>,
}

impl MailboxSender {
    /// Construct a sender.
    pub fn new(
        id: UnitId,
        responder: UnitId,
        cmd_mailbox: MailboxId,
        resp_mailbox: MailboxId,
        command: u32,
    ) -> Self {
        Self {
            id,
            responder,
            cmd_mailbox,
            resp_mailbox,
            command,
            phase: Cell::new(0),
        }
    }
}

impl ExecutionUnit for MailboxSender {
    type Snapshot = u8;
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.phase.get() >= 3 {
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
        let p = self.phase.get();
        self.phase.set(p + 1);
        match p {
            0 => {
                effects.push(Effect::MailboxSend {
                    mailbox: self.cmd_mailbox,
                    message: MailboxMessage::new(self.command),
                    source: self.id,
                });
                effects.push(Effect::WakeUnit {
                    target: self.responder,
                    source: self.id,
                });
                effects.push(Effect::WaitOnEvent {
                    target: cellgov_effects::WaitTarget::Mailbox(self.resp_mailbox),
                    source: self.id,
                });
                ExecutionStepResult {
                    yield_reason: YieldReason::MailboxAccess,
                    consumed_cost: InstructionCost::new(budget.raw()),
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                    syscall_args: None,
                }
            }
            1 => {
                effects.push(Effect::MailboxReceiveAttempt {
                    mailbox: self.resp_mailbox,
                    source: self.id,
                });
                ExecutionStepResult {
                    yield_reason: YieldReason::MailboxAccess,
                    consumed_cost: InstructionCost::new(budget.raw()),
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                    syscall_args: None,
                }
            }
            _ => {
                let response = ctx.received_messages().first().copied().unwrap_or(0);
                effects.push(Effect::TraceMarker {
                    marker: response,
                    source: self.id,
                });
                ExecutionStepResult {
                    yield_reason: YieldReason::Finished,
                    consumed_cost: InstructionCost::new(budget.raw()),
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                    syscall_args: None,
                }
            }
        }
    }
    fn snapshot(&self) -> u8 {
        self.phase.get()
    }
}

/// Two-stage SPU-like responder paired with [`MailboxSender`]: receive
/// attempt; then read command, send `command + 1` response, wake sender.
#[derive(Clone)]
pub struct MailboxResponder {
    id: UnitId,
    sender: UnitId,
    cmd_mailbox: MailboxId,
    resp_mailbox: MailboxId,
    phase: Cell<u8>,
}

impl MailboxResponder {
    /// Construct a responder.
    pub fn new(
        id: UnitId,
        sender: UnitId,
        cmd_mailbox: MailboxId,
        resp_mailbox: MailboxId,
    ) -> Self {
        Self {
            id,
            sender,
            cmd_mailbox,
            resp_mailbox,
            phase: Cell::new(0),
        }
    }
}

impl ExecutionUnit for MailboxResponder {
    type Snapshot = u8;
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.phase.get() >= 2 {
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
        let p = self.phase.get();
        self.phase.set(p + 1);
        match p {
            0 => {
                effects.push(Effect::MailboxReceiveAttempt {
                    mailbox: self.cmd_mailbox,
                    source: self.id,
                });
                ExecutionStepResult {
                    yield_reason: YieldReason::MailboxAccess,
                    consumed_cost: InstructionCost::new(budget.raw()),
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                    syscall_args: None,
                }
            }
            _ => {
                let cmd = ctx.received_messages().first().copied().unwrap_or(0);
                let response = cmd.wrapping_add(1);
                effects.push(Effect::MailboxSend {
                    mailbox: self.resp_mailbox,
                    message: MailboxMessage::new(response),
                    source: self.id,
                });
                effects.push(Effect::WakeUnit {
                    target: self.sender,
                    source: self.id,
                });
                ExecutionStepResult {
                    yield_reason: YieldReason::Finished,
                    consumed_cost: InstructionCost::new(budget.raw()),
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                    syscall_args: None,
                }
            }
        }
    }
    fn snapshot(&self) -> u8 {
        self.phase.get()
    }
}

#[cfg(test)]
#[path = "tests/world_tests.rs"]
mod tests;
