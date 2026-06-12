//! FakeIsaUnit program execution -- the yield and effect each opcode produces.

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
