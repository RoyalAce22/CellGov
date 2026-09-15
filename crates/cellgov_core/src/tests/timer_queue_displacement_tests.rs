use super::*;

#[test]
fn a_displacing_insert_returns_the_prior_wake_not_the_new_one() {
    let mut q = TimerWakeQueue::new();
    let unit = UnitId::new(2);
    assert_eq!(
        q.insert(GuestTicks::new(10), unit, TimerWakeKind::Sleep),
        None
    );

    let displaced = q.insert(
        GuestTicks::new(20),
        unit,
        TimerWakeKind::SyncWait(Lv2BlockReason::Mutex { id: 7 }),
    );

    assert_eq!(
        displaced,
        Some(TimerWake {
            unit,
            kind: TimerWakeKind::Sleep,
        }),
        "the caller names the lost wake in its invariant-break report, so insert \
         must hand back the prior entry, not the one that supersedes it",
    );
    assert_eq!(q.displacement_count(), 1);
}
