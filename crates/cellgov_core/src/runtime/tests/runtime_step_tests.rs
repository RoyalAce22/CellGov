//! Step/commit loop basics: scheduling order, budgets, epochs, and commit visibility.

use super::*;

#[test]
fn new_starts_at_zero() {
    let rt = build(16, 5, 100);
    assert_eq!(rt.time(), GuestTicks::ZERO);
    assert_eq!(rt.epoch(), Epoch::ZERO);
    assert_eq!(rt.steps_taken(), 0);
    assert_eq!(rt.max_steps(), 100);
    assert!(rt.registry().is_empty());
    assert_eq!(rt.memory().size(), 16);
}

#[test]
fn step_with_no_units_returns_no_runnable() {
    let mut rt = build(16, 5, 100);
    assert_eq!(rt.step().unwrap_err(), StepError::NoRunnableUnit);
    assert_eq!(rt.steps_taken(), 0);
}

#[test]
fn step_with_all_units_blocked_returns_all_blocked() {
    let mut rt = build(16, 5, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    rt.registry_mut()
        .set_status_override(UnitId::new(0), cellgov_exec::UnitStatus::Blocked);
    rt.registry_mut()
        .set_status_override(UnitId::new(1), cellgov_exec::UnitStatus::Blocked);
    assert_eq!(rt.step().unwrap_err(), StepError::AllBlocked);
    assert_eq!(rt.steps_taken(), 0);
}

#[test]
fn step_with_all_finished_returns_no_runnable_not_all_blocked() {
    let mut rt = build(16, 1, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    assert!(rt.step().is_ok());
    assert_eq!(rt.step().unwrap_err(), StepError::NoRunnableUnit);
}

#[test]
fn step_runs_a_registered_unit() {
    let mut rt = build(16, 5, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    let s = rt.step().unwrap();
    assert_eq!(s.unit, UnitId::new(0));
    assert_eq!(s.result.consumed_cost, InstructionCost::new(5));
    assert_eq!(s.time_after, GuestTicks::new(5));
    assert_eq!(rt.time(), GuestTicks::new(5));
    assert_eq!(rt.steps_taken(), 1);
}

#[test]
fn time_advances_by_consumed_cost() {
    let mut rt = build(16, 7, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    for i in 1..=4 {
        let s = rt.step().unwrap();
        assert_eq!(s.time_after, GuestTicks::new(7 * i));
        assert_eq!(rt.time(), GuestTicks::new(7 * i));
    }
}

#[test]
fn round_robin_visits_units_in_id_order() {
    let mut rt = build(16, 1, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    let ids: Vec<u64> = (0..6).map(|_| rt.step().unwrap().unit.raw()).collect();
    assert_eq!(ids, vec![0, 1, 2, 0, 1, 2]);
}

#[test]
fn finished_units_are_skipped() {
    let mut rt = build(16, 1, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 2));
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 5));
    let mut visited = Vec::new();
    for _ in 0..7 {
        match rt.step() {
            Ok(s) => visited.push(s.unit.raw()),
            Err(StepError::NoRunnableUnit) => break,
            Err(e) => panic!("unexpected error: {e:?}"),
        }
    }
    assert_eq!(visited, vec![0, 1, 0, 1, 1, 1, 1]);
    assert_eq!(rt.step().unwrap_err(), StepError::NoRunnableUnit);
}

#[test]
fn max_steps_cap_trips_deadlock_detector() {
    let mut rt = build(16, 1, 3);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 1000));
    assert!(rt.step().is_ok());
    assert!(rt.step().is_ok());
    assert!(rt.step().is_ok());
    assert_eq!(rt.step().unwrap_err(), StepError::MaxStepsExceeded);
    assert_eq!(rt.steps_taken(), 3);
}

#[test]
fn time_overflow_is_caught() {
    let mut rt = Runtime::new(GuestMemory::new(0), Budget::new(u64::MAX - 5), 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    let s = rt.step().unwrap();
    assert_eq!(s.time_after, GuestTicks::new(u64::MAX - 5));
    assert_eq!(rt.step().unwrap_err(), StepError::TimeOverflow);
}

#[test]
fn epoch_does_not_advance_within_a_single_step() {
    let mut rt = build(16, 1, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 5));
    for _ in 0..3 {
        let s = rt.step().unwrap();
        assert_eq!(s.epoch_after, Epoch::ZERO);
    }
    assert_eq!(rt.epoch(), Epoch::ZERO);
}

#[test]
fn step_returns_emitted_effects_in_order() {
    let mut rt = build(16, 1, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    let s = rt.step().unwrap();
    assert_eq!(s.effects.len(), 1);
    assert_eq!(
        s.effects[0],
        Effect::TraceMarker {
            marker: 1,
            source: UnitId::new(0),
        }
    );
}

#[test]
fn step_then_commit_writes_become_visible() {
    let mut rt = build(16, 1, 100);
    rt.registry_mut().register_with(|id| WritingUnit {
        id,
        steps: Cell::new(0),
        max: 5,
    });
    let s1 = rt.step().unwrap();
    let outcome1 = rt.commit_step(&s1.result, &s1.effects).unwrap();
    assert_eq!(outcome1.writes_committed, 1);
    assert!(!outcome1.fault_discarded);
    assert_eq!(
        rt.memory()
            .read(cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0), 4).unwrap())
            .unwrap(),
        &[1, 1, 1, 1]
    );
    assert_eq!(rt.epoch(), Epoch::new(1));

    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result, &s2.effects).unwrap();
    assert_eq!(
        rt.memory()
            .read(cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0), 4).unwrap())
            .unwrap(),
        &[2, 2, 2, 2]
    );
    assert_eq!(rt.epoch(), Epoch::new(2));
}

#[test]
fn max_steps_zero_rejects_first_step() {
    let mem = GuestMemory::new(64);
    let mut rt = Runtime::new(mem, Budget::new(10), 0);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 5));
    assert_eq!(rt.step(), Err(StepError::MaxStepsExceeded));
}

#[test]
fn into_memory_returns_committed_state() {
    let mut mem = GuestMemory::new(64);
    let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0), 4).unwrap();
    mem.apply_commit(range, &[0xDE, 0xAD, 0xBE, 0xEF]).unwrap();

    let rt = Runtime::new(mem, Budget::new(1), 100);
    let recovered = rt.into_memory();

    assert_eq!(&recovered.as_bytes()[0..4], &[0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(recovered.size(), 64);
}
