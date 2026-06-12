//! Synthetic world units -- counting, writing, mailbox, and signal -- and the effects each emits per step.

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
