//! Tiny fake ISA for pressure-testing the runtime contract.
//!
//! Each opcode maps to at least one distinct `Effect` so a single unit
//! type can exercise every path through the effect/commit pipeline.
//! Not a real instruction set and not tied to any PS3 architecture.

use crate::context::ExecutionContext;
use crate::step_result::ExecutionStepResult;
use crate::unit::{ExecutionUnit, UnitStatus};
use crate::yield_reason::YieldReason;
use crate::LocalDiagnostics;
use cellgov_effects::{Effect, MailboxMessage, WaitTarget, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_time::{Budget, GuestTicks};

/// A single fake-ISA opcode.
///
/// Atomic opcodes (`ReservationAcquire`, `ConditionalStore`) are
/// pass-throughs to their effect counterparts; the unit carries no
/// local reservation register, so the test harness is responsible
/// for driving the committed reservation table directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeOp {
    /// Load `value` into the accumulator. No effect emitted.
    LoadImm(u32),
    /// Emit `SharedWriteIntent` writing the accumulator's low byte
    /// (replicated) across the range.
    SharedStore {
        /// Start address.
        addr: u64,
        /// Byte count.
        len: u64,
    },
    /// Emit `Effect::ReservationAcquire` for the 128-byte line
    /// containing `line_addr`.
    ReservationAcquire {
        /// Byte address anywhere inside the line.
        line_addr: u64,
    },
    /// Emit `Effect::ConditionalStore` writing the accumulator's low
    /// byte (replicated) across the range. Assumes the reservation
    /// is held; the harness orders this against the table.
    ConditionalStore {
        /// Start address.
        addr: u64,
        /// Byte count. Must be 4, 8, or 128.
        len: u64,
    },
    /// Emit `MailboxSend` with the accumulator as the message word.
    MailboxSend {
        /// Target mailbox id.
        mailbox: u64,
    },
    /// Emit `MailboxReceiveAttempt`.
    MailboxRecv {
        /// Source mailbox id.
        mailbox: u64,
    },
    /// Emit `DmaEnqueue` (Put direction).
    DmaPut {
        /// Source address.
        src: u64,
        /// Destination address.
        dst: u64,
        /// Transfer size in bytes.
        len: u64,
    },
    /// Emit `WaitOnEvent` on a signal with the given mask.
    Wait {
        /// Signal id.
        signal: u64,
        /// Bit mask for signal matching.
        mask: u32,
    },
    /// Emit `WaitOnEvent` on a barrier.
    Barrier {
        /// Barrier id.
        barrier: u64,
    },
    /// Terminal: yield `Finished`.
    End,
}

/// Execution unit that interprets a `Vec<FakeOp>` program one opcode
/// per `run_until_yield`.
pub struct FakeIsaUnit {
    id: UnitId,
    program: Vec<FakeOp>,
    pc: usize,
    acc: u32,
    finished: bool,
}

impl FakeIsaUnit {
    /// Build a unit whose program starts at opcode 0.
    pub fn new(id: UnitId, program: Vec<FakeOp>) -> Self {
        Self {
            id,
            program,
            pc: 0,
            acc: 0,
            finished: false,
        }
    }

    /// Current program counter.
    pub fn pc(&self) -> usize {
        self.pc
    }

    /// Current accumulator value.
    pub fn acc(&self) -> u32 {
        self.acc
    }
}

impl ExecutionUnit for FakeIsaUnit {
    type Snapshot = (usize, u32);

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.finished {
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
        if let Some(&msg) = ctx.received_messages().first() {
            self.acc = msg;
        }

        if self.pc >= self.program.len() {
            self.finished = true;
            return ExecutionStepResult {
                yield_reason: YieldReason::Finished,
                consumed_budget: budget,
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
                syscall_args: None,
            };
        }

        let op = self.program[self.pc].clone();
        self.pc += 1;

        let yield_reason = match op {
            FakeOp::LoadImm(value) => {
                self.acc = value;
                YieldReason::BudgetExhausted
            }
            FakeOp::SharedStore { addr, len } => {
                let byte = self.acc as u8;
                let range = ByteRange::new(GuestAddr::new(addr), len)
                    .expect("SharedStore range must be valid");
                effects.push(Effect::SharedWriteIntent {
                    range,
                    bytes: WritePayload::new(vec![byte; len as usize]),
                    ordering: PriorityClass::Normal,
                    source: self.id,
                    source_time: GuestTicks::ZERO,
                });
                YieldReason::BudgetExhausted
            }
            FakeOp::MailboxSend { mailbox } => {
                effects.push(Effect::MailboxSend {
                    mailbox: cellgov_sync::MailboxId::new(mailbox),
                    message: MailboxMessage::new(self.acc),
                    source: self.id,
                });
                YieldReason::MailboxAccess
            }
            FakeOp::MailboxRecv { mailbox } => {
                effects.push(Effect::MailboxReceiveAttempt {
                    mailbox: cellgov_sync::MailboxId::new(mailbox),
                    source: self.id,
                });
                YieldReason::MailboxAccess
            }
            FakeOp::DmaPut { src, dst, len } => {
                let src_range = ByteRange::new(GuestAddr::new(src), len)
                    .expect("DmaPut src range must be valid");
                let dst_range = ByteRange::new(GuestAddr::new(dst), len)
                    .expect("DmaPut dst range must be valid");
                let req = cellgov_dma::DmaRequest::new(
                    cellgov_dma::DmaDirection::Put,
                    src_range,
                    dst_range,
                    self.id,
                )
                .expect("DmaPut src and dst lengths must match");
                effects.push(Effect::DmaEnqueue {
                    request: req,
                    payload: None,
                });
                YieldReason::DmaSubmitted
            }
            FakeOp::Wait { signal, mask: _ } => {
                effects.push(Effect::WaitOnEvent {
                    target: WaitTarget::Signal(cellgov_sync::SignalId::new(signal)),
                    source: self.id,
                });
                YieldReason::WaitingSync
            }
            FakeOp::Barrier { barrier } => {
                effects.push(Effect::WaitOnEvent {
                    target: WaitTarget::Barrier(cellgov_sync::BarrierId::new(barrier)),
                    source: self.id,
                });
                YieldReason::WaitingSync
            }
            FakeOp::ReservationAcquire { line_addr } => {
                effects.push(Effect::ReservationAcquire {
                    line_addr,
                    source: self.id,
                });
                YieldReason::BudgetExhausted
            }
            FakeOp::ConditionalStore { addr, len } => {
                let byte = self.acc as u8;
                let range = ByteRange::new(GuestAddr::new(addr), len)
                    .expect("ConditionalStore range must be valid");
                effects.push(Effect::ConditionalStore {
                    range,
                    bytes: WritePayload::new(vec![byte; len as usize]),
                    ordering: PriorityClass::Normal,
                    source: self.id,
                    source_time: GuestTicks::ZERO,
                });
                YieldReason::BudgetExhausted
            }
            FakeOp::End => {
                self.finished = true;
                YieldReason::Finished
            }
        };

        ExecutionStepResult {
            yield_reason,
            consumed_budget: budget,
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> (usize, u32) {
        (self.pc, self.acc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_mem::GuestMemory;

    fn ctx(mem: &GuestMemory) -> ExecutionContext<'_> {
        ExecutionContext::new(mem)
    }

    #[test]
    fn empty_program_finishes_immediately() {
        let mem = GuestMemory::new(16);
        let mut u = FakeIsaUnit::new(UnitId::new(0), vec![]);
        let mut effects = Vec::new();
        let r = u.run_until_yield(Budget::new(1), &ctx(&mem), &mut effects);
        assert_eq!(r.yield_reason, YieldReason::Finished);
        assert_eq!(u.status(), UnitStatus::Finished);
    }

    #[test]
    fn end_finishes() {
        let mem = GuestMemory::new(16);
        let mut u = FakeIsaUnit::new(UnitId::new(0), vec![FakeOp::End]);
        let mut effects = Vec::new();
        let r = u.run_until_yield(Budget::new(1), &ctx(&mem), &mut effects);
        assert_eq!(r.yield_reason, YieldReason::Finished);
        assert_eq!(u.status(), UnitStatus::Finished);
    }

    #[test]
    fn load_imm_sets_accumulator() {
        let mem = GuestMemory::new(16);
        let mut u = FakeIsaUnit::new(UnitId::new(0), vec![FakeOp::LoadImm(42), FakeOp::End]);
        let mut effects = Vec::new();
        let r = u.run_until_yield(Budget::new(1), &ctx(&mem), &mut effects);
        assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
        assert_eq!(u.acc(), 42);
        assert_eq!(effects.len(), 0);
    }

    #[test]
    fn shared_store_emits_write_intent() {
        let mem = GuestMemory::new(16);
        let mut u = FakeIsaUnit::new(
            UnitId::new(0),
            vec![
                FakeOp::LoadImm(0xab),
                FakeOp::SharedStore { addr: 0, len: 4 },
                FakeOp::End,
            ],
        );
        let mut effects = Vec::new();
        u.run_until_yield(Budget::new(1), &ctx(&mem), &mut effects);
        effects.clear();
        u.run_until_yield(Budget::new(1), &ctx(&mem), &mut effects);
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::SharedWriteIntent { bytes, .. } => {
                assert_eq!(bytes.bytes(), &[0xab, 0xab, 0xab, 0xab]);
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn mailbox_send_emits_effect_with_accumulator() {
        let mem = GuestMemory::new(16);
        let mut u = FakeIsaUnit::new(
            UnitId::new(0),
            vec![
                FakeOp::LoadImm(0xdead),
                FakeOp::MailboxSend { mailbox: 0 },
                FakeOp::End,
            ],
        );
        let mut effects = Vec::new();
        u.run_until_yield(Budget::new(1), &ctx(&mem), &mut effects);
        effects.clear();
        u.run_until_yield(Budget::new(1), &ctx(&mem), &mut effects);
        match &effects[0] {
            Effect::MailboxSend { message, .. } => {
                assert_eq!(message.raw(), 0xdead);
            }
            other => panic!("expected MailboxSend, got {other:?}"),
        }
    }

    #[test]
    fn mailbox_recv_emits_receive_attempt() {
        let mem = GuestMemory::new(16);
        let mut u = FakeIsaUnit::new(
            UnitId::new(0),
            vec![FakeOp::MailboxRecv { mailbox: 1 }, FakeOp::End],
        );
        let mut effects = Vec::new();
        u.run_until_yield(Budget::new(1), &ctx(&mem), &mut effects);
        assert!(matches!(effects[0], Effect::MailboxReceiveAttempt { .. }));
    }

    #[test]
    fn dma_put_emits_enqueue() {
        let mem = GuestMemory::new(256);
        let mut u = FakeIsaUnit::new(
            UnitId::new(0),
            vec![
                FakeOp::DmaPut {
                    src: 0,
                    dst: 128,
                    len: 16,
                },
                FakeOp::End,
            ],
        );
        let mut effects = Vec::new();
        let r = u.run_until_yield(Budget::new(1), &ctx(&mem), &mut effects);
        assert!(matches!(effects[0], Effect::DmaEnqueue { .. }));
        assert_eq!(r.yield_reason, YieldReason::DmaSubmitted);
    }

    #[test]
    fn wait_emits_wait_on_signal() {
        let mem = GuestMemory::new(16);
        let mut u = FakeIsaUnit::new(
            UnitId::new(0),
            vec![
                FakeOp::Wait {
                    signal: 7,
                    mask: 0x1,
                },
                FakeOp::End,
            ],
        );
        let mut effects = Vec::new();
        u.run_until_yield(Budget::new(1), &ctx(&mem), &mut effects);
        match &effects[0] {
            Effect::WaitOnEvent {
                target: WaitTarget::Signal(sig),
                ..
            } => {
                assert_eq!(sig.raw(), 7);
            }
            other => panic!("expected WaitOnEvent/Signal, got {other:?}"),
        }
    }

    #[test]
    fn barrier_emits_wait_on_barrier() {
        let mem = GuestMemory::new(16);
        let mut u = FakeIsaUnit::new(
            UnitId::new(0),
            vec![FakeOp::Barrier { barrier: 3 }, FakeOp::End],
        );
        let mut effects = Vec::new();
        u.run_until_yield(Budget::new(1), &ctx(&mem), &mut effects);
        match &effects[0] {
            Effect::WaitOnEvent {
                target: WaitTarget::Barrier(b),
                ..
            } => {
                assert_eq!(b.raw(), 3);
            }
            other => panic!("expected WaitOnEvent/Barrier, got {other:?}"),
        }
    }

    #[test]
    fn received_messages_load_into_accumulator() {
        let mem = GuestMemory::new(16);
        let mut u = FakeIsaUnit::new(UnitId::new(0), vec![FakeOp::End]);
        let received = vec![0xcafe_u32];
        let ctx = ExecutionContext::with_received(&mem, &received);
        let mut effects = Vec::new();
        u.run_until_yield(Budget::new(1), &ctx, &mut effects);
        assert_eq!(u.acc(), 0xcafe);
    }

    #[test]
    fn snapshot_captures_pc_and_acc() {
        let mem = GuestMemory::new(16);
        let mut u = FakeIsaUnit::new(UnitId::new(0), vec![FakeOp::LoadImm(99), FakeOp::End]);
        assert_eq!(u.snapshot(), (0, 0));
        let mut effects = Vec::new();
        u.run_until_yield(Budget::new(1), &ctx(&mem), &mut effects);
        assert_eq!(u.snapshot(), (1, 99));
    }

    #[test]
    fn multi_opcode_program_advances_pc_per_step() {
        let mem = GuestMemory::new(16);
        let mut u = FakeIsaUnit::new(
            UnitId::new(0),
            vec![
                FakeOp::LoadImm(1),
                FakeOp::LoadImm(2),
                FakeOp::LoadImm(3),
                FakeOp::End,
            ],
        );
        let mut effects = Vec::new();
        for expected_acc in [1, 2, 3] {
            effects.clear();
            u.run_until_yield(Budget::new(1), &ctx(&mem), &mut effects);
            assert_eq!(u.acc(), expected_acc);
        }
        effects.clear();
        let r = u.run_until_yield(Budget::new(1), &ctx(&mem), &mut effects);
        assert_eq!(r.yield_reason, YieldReason::Finished);
    }
}
