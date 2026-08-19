//! Timer-wake queue lifecycle, ordering, cancel index, and hash coverage.

use super::*;

fn tick(n: u64) -> GuestTicks {
    GuestTicks::new(n)
}

#[test]
fn new_queue_is_empty() {
    let q = TimerWakeQueue::new();
    assert!(q.is_empty());
    assert_eq!(q.len(), 0);
    assert_eq!(q.peek_deadline(), None);
}

#[test]
fn insert_and_pop_due() {
    let mut q = TimerWakeQueue::new();
    let unit = UnitId::new(3);
    q.insert(tick(100), unit, TimerWakeKind::Sleep);
    assert!(q.contains(unit));
    assert_eq!(q.peek_deadline(), Some(tick(100)));

    assert!(q.pop_due(tick(99)).is_empty());
    let due = q.pop_due(tick(100));
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].unit, unit);
    assert_eq!(due[0].kind, TimerWakeKind::Sleep);
    assert!(q.is_empty());
    assert!(!q.contains(unit));
}

#[test]
fn pop_due_drains_in_deadline_then_sequence_order() {
    let mut q = TimerWakeQueue::new();
    q.insert(tick(200), UnitId::new(1), TimerWakeKind::Sleep);
    q.insert(tick(100), UnitId::new(2), TimerWakeKind::Sleep);
    q.insert(tick(100), UnitId::new(3), TimerWakeKind::Sleep);

    let due = q.pop_due(tick(300));
    let units: Vec<u64> = due.iter().map(|w| w.unit.raw()).collect();
    // Equal deadlines drain in registration order (sequence tiebreak).
    assert_eq!(units, vec![2, 3, 1]);
}

#[test]
fn pop_due_leaves_later_deadlines() {
    let mut q = TimerWakeQueue::new();
    q.insert(tick(100), UnitId::new(1), TimerWakeKind::Sleep);
    q.insert(tick(200), UnitId::new(2), TimerWakeKind::Sleep);

    let due = q.pop_due(tick(150));
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].unit, UnitId::new(1));
    assert_eq!(q.len(), 1);
    assert_eq!(q.peek_deadline(), Some(tick(200)));
    assert!(q.contains(UnitId::new(2)));
}

#[test]
fn pop_due_at_u64_max_drains_everything() {
    let mut q = TimerWakeQueue::new();
    q.insert(tick(u64::MAX), UnitId::new(1), TimerWakeKind::Sleep);
    q.insert(tick(5), UnitId::new(2), TimerWakeKind::Sleep);
    let due = q.pop_due(tick(u64::MAX));
    assert_eq!(due.len(), 2);
    assert!(q.is_empty());
}

#[test]
fn cancel_removes_pending_wake() {
    let mut q = TimerWakeQueue::new();
    let unit = UnitId::new(7);
    q.insert(tick(100), unit, TimerWakeKind::Sleep);
    assert!(q.cancel(unit));
    assert!(q.is_empty());
    assert!(!q.contains(unit));
    assert!(q.pop_due(tick(1000)).is_empty());
}

#[test]
fn cancel_without_entry_returns_false() {
    let mut q = TimerWakeQueue::new();
    assert!(!q.cancel(UnitId::new(9)));
    q.insert(tick(1), UnitId::new(1), TimerWakeKind::Sleep);
    assert!(!q.cancel(UnitId::new(2)));
    assert_eq!(q.len(), 1);
}

#[test]
fn cancel_is_idempotent() {
    let mut q = TimerWakeQueue::new();
    let unit = UnitId::new(4);
    q.insert(tick(50), unit, TimerWakeKind::Sleep);
    assert!(q.cancel(unit));
    assert!(!q.cancel(unit));
}

#[cfg(debug_assertions)]
#[test]
fn insert_duplicate_unit_panics_in_debug_builds() {
    let mut q = TimerWakeQueue::new();
    let unit = UnitId::new(1);
    q.insert(tick(10), unit, TimerWakeKind::Sleep);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        q.insert(tick(20), unit, TimerWakeKind::Sleep);
    }));
    assert!(
        result.is_err(),
        "debug build must panic on duplicate insert; the debug_assert \
         is what keeps missed cancels from shipping"
    );
}

#[cfg(not(debug_assertions))]
#[test]
fn insert_duplicate_unit_supersedes_in_release_builds() {
    let mut q = TimerWakeQueue::new();
    let unit = UnitId::new(1);
    q.insert(tick(10), unit, TimerWakeKind::Sleep);
    q.insert(tick(20), unit, TimerWakeKind::Sleep);
    assert_eq!(q.len(), 1, "prior entry must be cancelled, not orphaned");
    assert_eq!(q.peek_deadline(), Some(tick(20)));
    assert_eq!(q.displacement_count(), 1);
    let due = q.pop_due(tick(20));
    assert_eq!(due.len(), 1);
    assert!(q.is_empty());
}

#[test]
fn sequence_survives_drain_so_later_inserts_stay_ordered() {
    let mut q = TimerWakeQueue::new();
    q.insert(tick(100), UnitId::new(1), TimerWakeKind::Sleep);
    q.pop_due(tick(100));
    // Same deadline as a hypothetical stale key; the monotonic
    // sequence keeps the new entry distinct.
    q.insert(tick(100), UnitId::new(2), TimerWakeKind::Sleep);
    let due = q.pop_due(tick(100));
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].unit, UnitId::new(2));
}

#[test]
fn state_hash_is_deterministic() {
    let mut a = TimerWakeQueue::new();
    let mut b = TimerWakeQueue::new();
    a.insert(tick(100), UnitId::new(1), TimerWakeKind::Sleep);
    b.insert(tick(100), UnitId::new(1), TimerWakeKind::Sleep);
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_covers_deadline_unit_and_count() {
    let mut base = TimerWakeQueue::new();
    base.insert(tick(100), UnitId::new(1), TimerWakeKind::Sleep);
    let base_hash = base.state_hash();

    let empty = TimerWakeQueue::new();
    assert_ne!(empty.state_hash(), base_hash);

    let mut other_deadline = TimerWakeQueue::new();
    other_deadline.insert(tick(200), UnitId::new(1), TimerWakeKind::Sleep);
    assert_ne!(other_deadline.state_hash(), base_hash);

    let mut other_unit = TimerWakeQueue::new();
    other_unit.insert(tick(100), UnitId::new(2), TimerWakeKind::Sleep);
    assert_ne!(other_unit.state_hash(), base_hash);

    let mut two = TimerWakeQueue::new();
    two.insert(tick(100), UnitId::new(1), TimerWakeKind::Sleep);
    two.insert(tick(300), UnitId::new(2), TimerWakeKind::Sleep);
    assert_ne!(two.state_hash(), base_hash);
}

/// The sequence component is part of the wire format: two queues
/// reaching the same (deadline, unit) set through different insert /
/// cancel histories hash differently, which is correct -- fire order
/// among equal deadlines depends on it.
#[test]
fn state_hash_covers_sequence_history() {
    let mut fresh = TimerWakeQueue::new();
    fresh.insert(tick(100), UnitId::new(1), TimerWakeKind::Sleep);

    let mut churned = TimerWakeQueue::new();
    churned.insert(tick(50), UnitId::new(9), TimerWakeKind::Sleep);
    churned.cancel(UnitId::new(9));
    churned.insert(tick(100), UnitId::new(1), TimerWakeKind::Sleep);

    assert_ne!(fresh.state_hash(), churned.state_hash());
}

#[test]
fn state_hash_stable_after_cancel_roundtrip_from_empty() {
    let mut q = TimerWakeQueue::new();
    let h_empty = q.state_hash();
    q.insert(tick(100), UnitId::new(1), TimerWakeKind::Sleep);
    assert_ne!(q.state_hash(), h_empty);
    // Cancel returns the ENTRY SET to empty but next_seq advanced;
    // the hash covers only live entries, so it returns to the empty
    // value -- entry-set equality is what replay comparison needs.
    q.cancel(UnitId::new(1));
    assert_eq!(q.state_hash(), h_empty);
}

#[test]
fn state_hash_distinguishes_wake_kind_and_block_reason() {
    let entries = [
        TimerWakeKind::Sleep,
        TimerWakeKind::SyncWait(Lv2BlockReason::Semaphore { id: 5 }),
        TimerWakeKind::SyncWait(Lv2BlockReason::Mutex { id: 5 }),
        TimerWakeKind::SyncWait(Lv2BlockReason::Semaphore { id: 6 }),
    ];
    let hashes: Vec<u64> = entries
        .iter()
        .map(|kind| {
            let mut q = TimerWakeQueue::new();
            q.insert(tick(100), UnitId::new(1), *kind);
            q.state_hash()
        })
        .collect();
    for (i, a) in hashes.iter().enumerate() {
        for (j, b) in hashes.iter().enumerate() {
            if i != j {
                assert_ne!(
                    a, b,
                    "kinds {:?} and {:?} must hash differently",
                    entries[i], entries[j]
                );
            }
        }
    }
}

/// Pinned FNV-1a value over a fixture that covers both kind tags and
/// every `Lv2BlockReason` variant.
#[test]
fn state_hash_wire_format_golden() {
    let mut q = TimerWakeQueue::new();
    q.insert(tick(1_000_000), UnitId::new(0), TimerWakeKind::Sleep);
    q.insert(
        tick(15_000_000_000),
        UnitId::new(3),
        TimerWakeKind::SyncWait(Lv2BlockReason::ThreadGroupJoin { group_id: 2 }),
    );
    q.insert(
        tick(1_000_000),
        UnitId::new(7),
        TimerWakeKind::SyncWait(Lv2BlockReason::PpuThreadJoin { target: 0x1_0000 }),
    );
    q.insert(
        tick(2_000_000),
        UnitId::new(8),
        TimerWakeKind::SyncWait(Lv2BlockReason::LwMutex { id: 5 }),
    );
    q.insert(
        tick(3_000_000),
        UnitId::new(9),
        TimerWakeKind::SyncWait(Lv2BlockReason::Mutex { id: 6 }),
    );
    q.insert(
        tick(4_000_000),
        UnitId::new(10),
        TimerWakeKind::SyncWait(Lv2BlockReason::Semaphore { id: 7 }),
    );
    q.insert(
        tick(5_000_000),
        UnitId::new(11),
        TimerWakeKind::SyncWait(Lv2BlockReason::EventQueue { id: 8 }),
    );
    q.insert(
        tick(6_000_000),
        UnitId::new(12),
        TimerWakeKind::SyncWait(Lv2BlockReason::EventFlag { id: 9 }),
    );
    q.insert(
        tick(7_000_000),
        UnitId::new(13),
        TimerWakeKind::SyncWait(Lv2BlockReason::Cond {
            id: 22,
            mutex_id: 21,
            mutex_kind: cellgov_lv2::CondMutexKind::Mutex,
        }),
    );
    const EXPECTED: u64 = 917_142_914_607_979_740;
    assert_eq!(
        q.state_hash(),
        EXPECTED,
        "TimerWakeQueue::state_hash wire format drifted; a versioned format \
         change must bump STATE_HASH_FORMAT_VERSION and update EXPECTED \
         in the same commit"
    );
}
