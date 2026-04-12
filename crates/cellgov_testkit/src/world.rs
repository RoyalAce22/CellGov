//! World builders -- declarative helpers for constructing a runtime,
//! registering units, seeding committed memory, queueing mailbox/DMA
//! state, and assigning initial budgets. Tests should read like setup,
//! not plumbing.
//!
//! Currently provides reusable fake [`ExecutionUnit`] implementations
//! that scenario fixtures can register without redeclaring the same
//! shapes in every test crate. Memory seeding, mailbox seeding, and
//! DMA seeding helpers will land as separate slices when their backing
//! state machines exist.
//!
//! The fake units here are clean-room runtime probes. They never go
//! away, even after real PPU/SPU translation lands.

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

/// Fake unit that consumes its full granted budget every step, emits a
/// single `TraceMarker` effect, and finishes after `max` steps.
///
/// The default general-purpose fake. Useful for fairness, scheduling, and
/// trace-shape tests where the actual effect content does not matter.
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

    /// Number of steps this unit has executed so far.
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
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        let yield_reason = if n >= self.max {
            YieldReason::Finished
        } else {
            YieldReason::BudgetExhausted
        };
        ExecutionStepResult {
            yield_reason,
            consumed_budget: budget,
            emitted_effects: vec![Effect::TraceMarker {
                marker: n as u32,
                source: self.id,
            }],
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
        }
    }
    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

/// Fake unit that emits one `SharedWriteIntent` per step against a
/// fixed range, writing the step number byte-replicated across the
/// range, then finishes after `max` steps.
///
/// Useful for commit-pipeline, hash-sensitivity, and replay tests.
/// The write target is configurable so multiple `WritingUnit`s can
/// coexist in the same scenario without colliding.
pub struct WritingUnit {
    id: UnitId,
    steps: Cell<u64>,
    max: u64,
    range: ByteRange,
}

impl WritingUnit {
    /// Construct a unit that writes into `range` once per step and
    /// finishes after `max` steps. The write payload is the step
    /// number cast to a byte and replicated across the range.
    pub fn new(id: UnitId, max: u64, range: ByteRange) -> Self {
        Self {
            id,
            steps: Cell::new(0),
            max,
            range,
        }
    }

    /// Convenience constructor: write 4 bytes at address 0.
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
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        let yield_reason = if n >= self.max {
            YieldReason::Finished
        } else {
            YieldReason::BudgetExhausted
        };
        let bytes = vec![n as u8; self.range.length() as usize];
        ExecutionStepResult {
            yield_reason,
            consumed_budget: budget,
            emitted_effects: vec![Effect::SharedWriteIntent {
                range: self.range,
                bytes: WritePayload::new(bytes),
                ordering: PriorityClass::Normal,
                source: self.id,
                source_time: GuestTicks::ZERO,
            }],
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
        }
    }
    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

/// Fake unit that emits one [`Effect::MailboxSend`] per step into a
/// configured mailbox, with sequential message words `1..=max`, then
/// finishes after `max` steps.
///
/// Useful for end-to-end mailbox-send pipeline tests through the
/// runner. The mailbox id is configured at construction time so the
/// fixture decides which mailbox the producer feeds.
pub struct MailboxProducer {
    id: UnitId,
    target: MailboxId,
    steps: Cell<u64>,
    max: u64,
}

impl MailboxProducer {
    /// Construct a producer that sends `max` messages into `target`.
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
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        let yield_reason = if n >= self.max {
            YieldReason::Finished
        } else {
            YieldReason::MailboxAccess
        };
        ExecutionStepResult {
            yield_reason,
            consumed_budget: budget,
            emitted_effects: vec![Effect::MailboxSend {
                mailbox: self.target,
                message: MailboxMessage::new(n as u32),
                source: self.id,
            }],
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
        }
    }
    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

/// Fake unit that emits one [`Effect::SignalUpdate`] per step into a
/// configured signal register, OR-ing in a unique bit on each step
/// (`1 << (step - 1)`), then finishes after `bit_count` steps.
///
/// After `bit_count` steps, the target register's value is
/// `(1 << bit_count) - 1` -- the low `bit_count` bits set. Useful for
/// end-to-end signal pipeline tests through the runner. The signal id
/// is configured at construction time so the fixture decides which
/// register the emitter feeds.
///
/// `bit_count` must be at most 32 (the register is `u32`); the
/// constructor panics on larger values rather than silently wrapping.
pub struct SignalEmitter {
    id: UnitId,
    target: SignalId,
    steps: Cell<u64>,
    bit_count: u64,
}

impl SignalEmitter {
    /// Construct an emitter that performs `bit_count` OR-merges into
    /// `target`, one per step.
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
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        let yield_reason = if n >= self.bit_count {
            YieldReason::Finished
        } else {
            YieldReason::WaitingSync
        };
        // Bit `n - 1` so the first step OR-s in 0x1, second 0x2, etc.
        let value = 1u32 << (n - 1) as u32;
        ExecutionStepResult {
            yield_reason,
            consumed_budget: budget,
            emitted_effects: vec![Effect::SignalUpdate {
                signal: self.target,
                value,
                source: self.id,
            }],
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
        }
    }
    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

/// Fake unit for Scenario B. Seeds source bytes into committed
/// memory, submits a DMA Put transfer, and blocks. When the
/// completion fires (at `now + latency`), the runtime wakes this
/// unit. On wake it emits a `TraceMarker` and finishes.
///
/// Two phases: Seed+Submit+Block, then Wake+Finish.
pub struct DmaSubmitter {
    id: UnitId,
    source: ByteRange,
    destination: ByteRange,
    seed_bytes: Vec<u8>,
    phase: Cell<u8>,
}

impl DmaSubmitter {
    /// Construct a submitter that writes `seed_bytes` to `source`,
    /// then enqueues a DMA Put from `source` to `destination`.
    /// `seed_bytes.len()` must equal `source.length()` and
    /// `destination.length()`.
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
    ) -> ExecutionStepResult {
        let p = self.phase.get();
        self.phase.set(p + 1);
        match p {
            0 => {
                // Phase 0: seed source bytes, submit DMA, block.
                let req =
                    DmaRequest::new(DmaDirection::Put, self.source, self.destination, self.id)
                        .expect("source and destination lengths match");
                ExecutionStepResult {
                    yield_reason: YieldReason::DmaSubmitted,
                    consumed_budget: budget,
                    emitted_effects: vec![
                        Effect::SharedWriteIntent {
                            range: self.source,
                            bytes: WritePayload::new(self.seed_bytes.clone()),
                            ordering: PriorityClass::Normal,
                            source: self.id,
                            source_time: GuestTicks::ZERO,
                        },
                        Effect::DmaEnqueue {
                            request: req,
                            payload: None,
                        },
                        Effect::WaitOnEvent {
                            target: cellgov_effects::WaitTarget::Barrier(
                                cellgov_sync::BarrierId::new(0),
                            ),
                            source: self.id,
                        },
                    ],
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                }
            }
            _ => {
                // Woken after DMA completed. Marker + finish.
                ExecutionStepResult {
                    yield_reason: YieldReason::Finished,
                    consumed_budget: budget,
                    emitted_effects: vec![Effect::TraceMarker {
                        marker: 0xd0d0,
                        source: self.id,
                    }],
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                }
            }
        }
    }
    fn snapshot(&self) -> u8 {
        self.phase.get()
    }
}

/// Fake "PPU" unit for Scenario A. Sends a command word to
/// `cmd_mailbox`, wakes `responder`, blocks itself, then -- on its
/// next scheduled step -- receives the response from `resp_mailbox`
/// and finishes with a `TraceMarker` carrying the response value.
///
/// Currently requires explicit `WakeUnit` because the commit pipeline
/// does not auto-wake on message delivery. The three internal phases
/// are: Send (emit send + wake + wait), Receive (emit receive
/// attempt), Consume (read `received_messages`, emit marker, finish).
pub struct MailboxSender {
    id: UnitId,
    responder: UnitId,
    cmd_mailbox: MailboxId,
    resp_mailbox: MailboxId,
    command: u32,
    phase: Cell<u8>,
}

impl MailboxSender {
    /// Create a new mailbox sender unit.
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
    ) -> ExecutionStepResult {
        let p = self.phase.get();
        self.phase.set(p + 1);
        match p {
            0 => {
                // Phase 0: send command, wake responder, block self.
                ExecutionStepResult {
                    yield_reason: YieldReason::MailboxAccess,
                    consumed_budget: budget,
                    emitted_effects: vec![
                        Effect::MailboxSend {
                            mailbox: self.cmd_mailbox,
                            message: MailboxMessage::new(self.command),
                            source: self.id,
                        },
                        Effect::WakeUnit {
                            target: self.responder,
                            source: self.id,
                        },
                        Effect::WaitOnEvent {
                            target: cellgov_effects::WaitTarget::Mailbox(self.resp_mailbox),
                            source: self.id,
                        },
                    ],
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                }
            }
            1 => {
                // Attempt to receive response.
                ExecutionStepResult {
                    yield_reason: YieldReason::MailboxAccess,
                    consumed_budget: budget,
                    emitted_effects: vec![Effect::MailboxReceiveAttempt {
                        mailbox: self.resp_mailbox,
                        source: self.id,
                    }],
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                }
            }
            _ => {
                // Consume the received response and finish.
                let response = ctx.received_messages().first().copied().unwrap_or(0);
                ExecutionStepResult {
                    yield_reason: YieldReason::Finished,
                    consumed_budget: budget,
                    emitted_effects: vec![Effect::TraceMarker {
                        marker: response,
                        source: self.id,
                    }],
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                }
            }
        }
    }
    fn snapshot(&self) -> u8 {
        self.phase.get()
    }
}

/// Fake "SPU" unit for Scenario A. Receives a command from
/// `cmd_mailbox`, computes a response (`command + 1`), sends it to
/// `resp_mailbox`, wakes the `sender`, and finishes.
///
/// Two internal phases: Receive (emit receive attempt), Respond
/// (read received, emit send + wake, finish).
pub struct MailboxResponder {
    id: UnitId,
    sender: UnitId,
    cmd_mailbox: MailboxId,
    resp_mailbox: MailboxId,
    phase: Cell<u8>,
}

impl MailboxResponder {
    /// Create a new mailbox responder unit.
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
    ) -> ExecutionStepResult {
        let p = self.phase.get();
        self.phase.set(p + 1);
        match p {
            0 => {
                // Phase 0: attempt to receive command.
                ExecutionStepResult {
                    yield_reason: YieldReason::MailboxAccess,
                    consumed_budget: budget,
                    emitted_effects: vec![Effect::MailboxReceiveAttempt {
                        mailbox: self.cmd_mailbox,
                        source: self.id,
                    }],
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                }
            }
            _ => {
                // Read command, send response, wake sender, finish.
                let cmd = ctx.received_messages().first().copied().unwrap_or(0);
                let response = cmd.wrapping_add(1);
                ExecutionStepResult {
                    yield_reason: YieldReason::Finished,
                    consumed_budget: budget,
                    emitted_effects: vec![
                        Effect::MailboxSend {
                            mailbox: self.resp_mailbox,
                            message: MailboxMessage::new(response),
                            source: self.id,
                        },
                        Effect::WakeUnit {
                            target: self.sender,
                            source: self.id,
                        },
                    ],
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
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
        for i in 1..=3 {
            assert_eq!(u.status(), UnitStatus::Runnable);
            let r = u.run_until_yield(Budget::new(1), &ctx);
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
        let r = u.run_until_yield(Budget::new(1), &ctx);
        assert_eq!(r.emitted_effects.len(), 1);
        match &r.emitted_effects[0] {
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

        let r1 = u.run_until_yield(Budget::new(1), &ctx);
        match &r1.emitted_effects[0] {
            Effect::MailboxSend {
                mailbox, message, ..
            } => {
                assert_eq!(*mailbox, target);
                assert_eq!(message.raw(), 1);
            }
            other => panic!("expected MailboxSend, got {other:?}"),
        }
        assert_eq!(r1.yield_reason, YieldReason::MailboxAccess);

        let _ = u.run_until_yield(Budget::new(1), &ctx);
        let r3 = u.run_until_yield(Budget::new(1), &ctx);
        match &r3.emitted_effects[0] {
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

        // Step 1 -> 0x1, step 2 -> 0x2, step 3 -> 0x4, step 4 -> 0x8.
        for (i, expected_bit) in [1u32, 2, 4, 8].iter().enumerate() {
            assert_eq!(u.status(), UnitStatus::Runnable);
            let r = u.run_until_yield(Budget::new(1), &ctx);
            match &r.emitted_effects[0] {
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
        let r = u.run_until_yield(Budget::new(1), &ctx);
        match &r.emitted_effects[0] {
            Effect::SharedWriteIntent { range, .. } => {
                assert_eq!(range.start(), GuestAddr::new(0));
                assert_eq!(range.length(), 4);
            }
            _ => unreachable!(),
        }
    }
}
