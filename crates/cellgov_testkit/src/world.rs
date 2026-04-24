//! Reusable fake [`ExecutionUnit`] implementations composed by scenario
//! fixtures.
//!
//! The fakes probe runtime wiring directly, independent of any real
//! architectural interpreter.

use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::{Effect, MailboxMessage, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_sync::{MailboxId, SignalId};
use cellgov_time::{Budget, GuestTicks};
use std::cell::Cell;

/// Consumes its full budget each step, emits one `TraceMarker`, finishes
/// after `max` steps.
pub struct CountingUnit {
    id: UnitId,
    steps: Cell<u64>,
    max: u64,
}

impl CountingUnit {
    /// Construct a unit that finishes after `max` steps.
    pub fn new(id: UnitId, max: u64) -> Self {
        Self {
            id,
            steps: Cell::new(0),
            max,
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
        ExecutionStepResult {
            yield_reason,
            consumed_budget: budget,
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }
    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

/// Emits one `SharedWriteIntent` per step against `range`, payload is
/// the step number byte-replicated; finishes after `max` steps.
pub struct WritingUnit {
    id: UnitId,
    steps: Cell<u64>,
    max: u64,
    range: ByteRange,
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
        let bytes = vec![n as u8; self.range.length() as usize];
        effects.push(Effect::SharedWriteIntent {
            range: self.range,
            bytes: WritePayload::new(bytes),
            ordering: PriorityClass::Normal,
            source: self.id,
            source_time: GuestTicks::ZERO,
        });
        ExecutionStepResult {
            yield_reason,
            consumed_budget: budget,
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
            consumed_budget: budget,
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
            consumed_budget: budget,
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
                effects.push(Effect::SharedWriteIntent {
                    range: self.source,
                    bytes: WritePayload::new(self.seed_bytes.clone()),
                    ordering: PriorityClass::Normal,
                    source: self.id,
                    source_time: GuestTicks::ZERO,
                });
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
                    consumed_budget: budget,
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
                    consumed_budget: budget,
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
                    consumed_budget: budget,
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
                    consumed_budget: budget,
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
                    consumed_budget: budget,
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
                    consumed_budget: budget,
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
                    consumed_budget: budget,
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
mod tests {
    use super::*;
    use cellgov_mem::GuestMemory;

    #[test]
    fn counting_unit_finishes_after_max_steps() {
        let mem = GuestMemory::new(8);
        let ctx = ExecutionContext::new(&mem);
        let mut u = CountingUnit::new(UnitId::new(0), 3);
        let mut effects = Vec::new();
        for i in 1..=3 {
            assert_eq!(u.status(), UnitStatus::Runnable);
            effects.clear();
            let r = u.run_until_yield(Budget::new(1), &ctx, &mut effects);
            assert_eq!(u.steps_taken(), i);
            if i == 3 {
                assert_eq!(r.yield_reason, YieldReason::Finished);
            } else {
                assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
            }
        }
        assert_eq!(u.status(), UnitStatus::Finished);
    }

    #[test]
    fn writing_unit_emits_shared_write_intent_with_step_payload() {
        let mem = GuestMemory::new(16);
        let ctx = ExecutionContext::new(&mem);
        let range = ByteRange::new(GuestAddr::new(4), 4).unwrap();
        let mut u = WritingUnit::new(UnitId::new(0), 2, range);
        let mut effects = Vec::new();
        u.run_until_yield(Budget::new(1), &ctx, &mut effects);
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::SharedWriteIntent {
                range: r2, bytes, ..
            } => {
                assert_eq!(*r2, range);
                assert_eq!(bytes.bytes(), &[1, 1, 1, 1]);
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn mailbox_producer_emits_send_with_sequential_message_words() {
        let mem = GuestMemory::new(8);
        let ctx = ExecutionContext::new(&mem);
        let target = MailboxId::new(0);
        let mut u = MailboxProducer::new(UnitId::new(0), target, 3);
        let mut effects = Vec::new();

        let r1 = u.run_until_yield(Budget::new(1), &ctx, &mut effects);
        match &effects[0] {
            Effect::MailboxSend {
                mailbox, message, ..
            } => {
                assert_eq!(*mailbox, target);
                assert_eq!(message.raw(), 1);
            }
            other => panic!("expected MailboxSend, got {other:?}"),
        }
        assert_eq!(r1.yield_reason, YieldReason::MailboxAccess);

        effects.clear();
        let _ = u.run_until_yield(Budget::new(1), &ctx, &mut effects);
        effects.clear();
        let r3 = u.run_until_yield(Budget::new(1), &ctx, &mut effects);
        match &effects[0] {
            Effect::MailboxSend { message, .. } => {
                assert_eq!(message.raw(), 3);
            }
            _ => unreachable!(),
        }
        assert_eq!(r3.yield_reason, YieldReason::Finished);
        assert_eq!(u.status(), UnitStatus::Finished);
    }

    #[test]
    fn signal_emitter_or_merges_one_bit_per_step() {
        let mem = GuestMemory::new(8);
        let ctx = ExecutionContext::new(&mem);
        let target = SignalId::new(0);
        let mut u = SignalEmitter::new(UnitId::new(0), target, 4);
        let mut effects = Vec::new();

        // Step 1 -> 0x1, step 2 -> 0x2, step 3 -> 0x4, step 4 -> 0x8.
        for (i, expected_bit) in [1u32, 2, 4, 8].iter().enumerate() {
            assert_eq!(u.status(), UnitStatus::Runnable);
            effects.clear();
            let r = u.run_until_yield(Budget::new(1), &ctx, &mut effects);
            match &effects[0] {
                Effect::SignalUpdate { signal, value, .. } => {
                    assert_eq!(*signal, target);
                    assert_eq!(*value, *expected_bit);
                }
                other => panic!("expected SignalUpdate, got {other:?}"),
            }
            if i == 3 {
                assert_eq!(r.yield_reason, YieldReason::Finished);
            } else {
                assert_eq!(r.yield_reason, YieldReason::WaitingSync);
            }
        }
        assert_eq!(u.status(), UnitStatus::Finished);
    }

    #[test]
    #[should_panic(expected = "bit_count must be <= 32")]
    fn signal_emitter_more_than_32_bits_panics() {
        let _ = SignalEmitter::new(UnitId::new(0), SignalId::new(0), 33);
    }

    #[test]
    fn writing_unit_at_zero_writes_to_addr_zero() {
        let mem = GuestMemory::new(8);
        let ctx = ExecutionContext::new(&mem);
        let mut u = WritingUnit::at_zero(UnitId::new(0), 1);
        let mut effects = Vec::new();
        u.run_until_yield(Budget::new(1), &ctx, &mut effects);
        match &effects[0] {
            Effect::SharedWriteIntent { range, .. } => {
                assert_eq!(range.start(), GuestAddr::new(0));
                assert_eq!(range.length(), 4);
            }
            _ => unreachable!(),
        }
    }
}
