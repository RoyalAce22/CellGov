//! Guest block-reason tests -- stable-tag injectivity and Blocked-state round-trips.

use super::*;
use crate::ppu_thread::{PpuThreadAttrs, PpuThreadState, PpuThreadTable};
use cellgov_event::UnitId;

fn dummy_attrs() -> PpuThreadAttrs {
    PpuThreadAttrs {
        entry: 0x10_0000,
        arg: 0,
        stack_base: 0xD000_0000,
        stack_size: 0x10000,
        priority: 1000,
        tls_base: 0x0020_0000,
    }
}

#[test]
fn event_flag_wait_mode_stable_tag_is_injective() {
    let tags = [
        EventFlagWaitMode::AndNoClear.stable_tag(),
        EventFlagWaitMode::AndClear.stable_tag(),
        EventFlagWaitMode::OrNoClear.stable_tag(),
        EventFlagWaitMode::OrClear.stable_tag(),
    ];
    let mut seen = std::collections::BTreeSet::new();
    for t in tags {
        assert!(seen.insert(t), "duplicate stable_tag value: {t}");
    }
    assert_eq!(seen.len(), 4);
}

#[test]
fn guest_block_reason_stable_tag_is_injective_and_nonzero() {
    let reasons = [
        GuestBlockReason::WaitingOnJoin {
            target: PpuThreadId::PRIMARY,
        },
        GuestBlockReason::WaitingOnLwMutex { id: 1 },
        GuestBlockReason::WaitingOnMutex { id: 1 },
        GuestBlockReason::WaitingOnSemaphore { id: 1 },
        GuestBlockReason::WaitingOnEventQueue { id: 1 },
        GuestBlockReason::WaitingOnEventFlag {
            id: 1,
            mask: 0,
            mode: EventFlagWaitMode::AndNoClear,
        },
        GuestBlockReason::WaitingOnCond {
            cond_id: 1,
            mutex_id: 1,
        },
    ];
    let mut seen = std::collections::BTreeSet::new();
    for r in reasons {
        let t = r.stable_tag();
        assert_ne!(t, 0, "reason tag collides with Runnable lifecycle tag");
        assert!(seen.insert(t), "duplicate reason tag {t}");
    }
    assert_eq!(seen.len(), 7);
}

#[test]
fn blocked_state_carries_guest_reason() {
    let mut t = PpuThreadTable::new();
    let waiter = t.create(UnitId::new(2), dummy_attrs()).unwrap();
    let target = t.create(UnitId::new(3), dummy_attrs()).unwrap();
    t.get_mut(waiter).unwrap().state =
        PpuThreadState::Blocked(GuestBlockReason::WaitingOnJoin { target });
    match &t.get(waiter).unwrap().state {
        PpuThreadState::Blocked(GuestBlockReason::WaitingOnJoin { target: tgt }) => {
            assert_eq!(*tgt, target);
        }
        other => panic!("expected WaitingOnJoin, got {other:?}"),
    }
}

#[test]
fn all_guest_block_reason_variants_round_trip_through_blocked_state() {
    let mut t = PpuThreadTable::new();
    let waiter = t.create(UnitId::new(2), dummy_attrs()).unwrap();
    let reasons = [
        GuestBlockReason::WaitingOnJoin {
            target: PpuThreadId::PRIMARY,
        },
        GuestBlockReason::WaitingOnLwMutex { id: 7 },
        GuestBlockReason::WaitingOnMutex { id: 7 },
        GuestBlockReason::WaitingOnSemaphore { id: 7 },
        GuestBlockReason::WaitingOnEventQueue { id: 7 },
        GuestBlockReason::WaitingOnEventFlag {
            id: 7,
            mask: 0xF0F0,
            mode: EventFlagWaitMode::AndClear,
        },
        GuestBlockReason::WaitingOnCond {
            cond_id: 7,
            mutex_id: 8,
        },
    ];
    for reason in reasons {
        t.get_mut(waiter).unwrap().state = PpuThreadState::Blocked(reason);
        match &t.get(waiter).unwrap().state {
            PpuThreadState::Blocked(stored) => assert_eq!(*stored, reason),
            other => panic!("expected Blocked, got {other:?}"),
        }
    }
}
