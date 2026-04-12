//! Tiny fake ISA for pressure-testing runtime seams.
//!
//! Each opcode maps to at least one distinct `Effect` path so the
//! fake ISA exercises the full effect/commit pipeline from a single
//! unit type. This is not a real instruction set; it exists so the
//! runtime contract can be validated under realistic multi-effect
//! workloads before real PPC/SPU translation lands.
//!
//! A `FakeIsaUnit` holds a `Vec<FakeOp>` program and a program
//! counter. Each `run_until_yield` call decodes one opcode, emits the
//! corresponding effect(s), and advances the PC. When the PC reaches
//! `End` or runs off the end of the program, the unit yields
//! `Finished`.

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
/// The variant set is: `LoadImm`, `SharedStore`, `MailboxSend`,
/// `MailboxRecv`, `DmaPut`, `Wait`, `Barrier`, `End`. Each opcode maps
/// to at least one `Effect` variant so the pipeline sees every path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeOp {
    /// Load `value` into the unit's accumulator register.
    /// No effect emitted; yields `BudgetExhausted`.
    LoadImm(u32),
    /// Emit `SharedWriteIntent` writing the accumulator's low byte
    /// (replicated) across the given address range.
    SharedStore { addr: u64, len: u64 },
    /// Emit `MailboxSend` with the accumulator as the message word.
    MailboxSend { mailbox: u64 },
    /// Emit `MailboxReceiveAttempt`. The commit pipeline pops from
    /// the mailbox if non-empty or blocks the unit if empty.
    MailboxRecv { mailbox: u64 },
    /// Emit `DmaEnqueue` (Put direction, `src` -> `dst`, `len` bytes).
    DmaPut { src: u64, dst: u64, len: u64 },
    /// Emit `WaitOnEvent` on a signal with the given mask.
    Wait { signal: u64, mask: u32 },
    /// Emit `WaitOnEvent` on a barrier.
    Barrier { barrier: u64 },
    /// Yield `Finished`. Terminal opcode.
    End,
}

/// A fake execution unit that decodes a `Vec<FakeOp>` program.
///
/// One opcode per `run_until_yield` call. The unit owns a single
/// `u32` accumulator (`acc`) that `LoadImm` writes and other opcodes
/// read. The program counter (`pc`) advances by one per step; running
/// off the end or hitting `End` yields `Finished`.
pub struct FakeIsaUnit {
    id: UnitId,
    program: Vec<FakeOp>,
    pc: usize,
    acc: u32,
    finished: bool,
}

impl FakeIsaUnit {
    /// Construct a unit with the given program. Execution starts at
    /// opcode 0.
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
    ) -> ExecutionStepResult {
        // If any received messages are pending (from a prior
        // MailboxRecv), load the first into the accumulator.
        if let Some(&msg) = ctx.received_messages().first() {
            self.acc = msg;
        }

        if self.pc >= self.program.len() {
            self.finished = true;
            return ExecutionStepResult {
                yield_reason: YieldReason::Finished,
                consumed_budget: budget,
                emitted_effects: vec![],
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
            };
        }

        let op = self.program[self.pc].clone();
        self.pc += 1;

        let (yield_reason, effects) = match op {
            FakeOp::LoadImm(value) => {
                self.acc = value;
                (YieldReason::BudgetExhausted, vec![])
            }
            FakeOp::SharedStore { addr, len } => {
                let byte = self.acc as u8;
                let range = ByteRange::new(GuestAddr::new(addr), len)
                    .expect("SharedStore range must be valid");
                (
                    YieldReason::BudgetExhausted,
                    vec![Effect::SharedWriteIntent {
                        range,
                        bytes: WritePayload::new(vec![byte; len as usize]),
                        ordering: PriorityClass::Normal,
                        source: self.id,
                        source_time: GuestTicks::ZERO,
                    }],
                )
            }
            FakeOp::MailboxSend { mailbox } => (
                YieldReason::MailboxAccess,
                vec![Effect::MailboxSend {
                    mailbox: cellgov_sync::MailboxId::new(mailbox),
                    message: MailboxMessage::new(self.acc),
                    source: self.id,
                }],
            ),
            FakeOp::MailboxRecv { mailbox } => (
                YieldReason::MailboxAccess,
                vec![Effect::MailboxReceiveAttempt {
                    mailbox: cellgov_sync::MailboxId::new(mailbox),
                    source: self.id,
                }],
            ),
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
                (
                    YieldReason::DmaSubmitted,
                    vec![Effect::DmaEnqueue { request: req }],
                )
            }
            FakeOp::Wait { signal, mask: _ } => (
                YieldReason::WaitingSync,
                vec![Effect::WaitOnEvent {
                    target: WaitTarget::Signal(cellgov_sync::SignalId::new(signal)),
                    source: self.id,
                }],
            ),
            FakeOp::Barrier { barrier } => (
                YieldReason::WaitingSync,
                vec![Effect::WaitOnEvent {
                    target: WaitTarget::Barrier(cellgov_sync::BarrierId::new(barrier)),
                    source: self.id,
                }],
            ),
            FakeOp::End => {
                self.finished = true;
                (YieldReason::Finished, vec![])
            }
        };

        ExecutionStepResult {
            yield_reason,
            consumed_budget: budget,
            emitted_effects: effects,
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
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
        let r = u.run_until_yield(Budget::new(1), &ctx(&mem));
        assert_eq!(r.yield_reason, YieldReason::Finished);
        assert_eq!(u.status(), UnitStatus::Finished);
    }

    #[test]
    fn end_finishes() {
        let mem = GuestMemory::new(16);
        let mut u = FakeIsaUnit::new(UnitId::new(0), vec![FakeOp::End]);
        let r = u.run_until_yield(Budget::new(1), &ctx(&mem));
        assert_eq!(r.yield_reason, YieldReason::Finished);
        assert_eq!(u.status(), UnitStatus::Finished);
    }

    #[test]
    fn load_imm_sets_accumulator() {
        let mem = GuestMemory::new(16);
        let mut u = FakeIsaUnit::new(UnitId::new(0), vec![FakeOp::LoadImm(42), FakeOp::End]);
        let r = u.run_until_yield(Budget::new(1), &ctx(&mem));
        assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
        assert_eq!(u.acc(), 42);
        assert_eq!(r.emitted_effects.len(), 0);
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
        u.run_until_yield(Budget::new(1), &ctx(&mem)); // LoadImm
        let r = u.run_until_yield(Budget::new(1), &ctx(&mem)); // SharedStore
        assert_eq!(r.emitted_effects.len(), 1);
        match &r.emitted_effects[0] {
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
        u.run_until_yield(Budget::new(1), &ctx(&mem));
        let r = u.run_until_yield(Budget::new(1), &ctx(&mem));
        match &r.emitted_effects[0] {
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
        let r = u.run_until_yield(Budget::new(1), &ctx(&mem));
        assert!(matches!(
            r.emitted_effects[0],
            Effect::MailboxReceiveAttempt { .. }
        ));
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
        let r = u.run_until_yield(Budget::new(1), &ctx(&mem));
        assert!(matches!(r.emitted_effects[0], Effect::DmaEnqueue { .. }));
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
        let r = u.run_until_yield(Budget::new(1), &ctx(&mem));
        match &r.emitted_effects[0] {
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
        let r = u.run_until_yield(Budget::new(1), &ctx(&mem));
        match &r.emitted_effects[0] {
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
        // Simulate the runtime delivering a message.
        let received = vec![0xcafe_u32];
        let ctx = ExecutionContext::with_received(&mem, &received);
        u.run_until_yield(Budget::new(1), &ctx);
        assert_eq!(u.acc(), 0xcafe);
    }

    #[test]
    fn snapshot_captures_pc_and_acc() {
        let mem = GuestMemory::new(16);
        let mut u = FakeIsaUnit::new(UnitId::new(0), vec![FakeOp::LoadImm(99), FakeOp::End]);
        assert_eq!(u.snapshot(), (0, 0));
        u.run_until_yield(Budget::new(1), &ctx(&mem));
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
        for expected_acc in [1, 2, 3] {
            u.run_until_yield(Budget::new(1), &ctx(&mem));
            assert_eq!(u.acc(), expected_acc);
        }
        let r = u.run_until_yield(Budget::new(1), &ctx(&mem));
        assert_eq!(r.yield_reason, YieldReason::Finished);
    }
}
