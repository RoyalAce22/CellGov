//! The PPE sibling count in the dependency relation: a PPE thread's
//! step costs more while another PPE thread is runnable, so a step that
//! parks, wakes, creates or finishes one decides it.

use crate::dependency::StepFootprint;
use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

/// A footprint of a step PPE thread `unit` took.
fn read_by(unit: u64) -> StepFootprint {
    StepFootprint {
        ppe_sibling_reader: Some(UnitId::new(unit)),
        ..StepFootprint::default()
    }
}

/// A footprint of a step that moved PPE thread `unit` into or out of the
/// runnable set.
fn moved(unit: u64) -> StepFootprint {
    StepFootprint {
        ppe_status_changes: vec![UnitId::new(unit)],
        ..StepFootprint::default()
    }
}

#[test]
fn a_ppe_step_pairs_with_a_step_that_moves_another_ppe_thread_only() {
    assert!(read_by(0).conflicts(&moved(1)));
    assert!(moved(1).conflicts(&read_by(0)));
    assert!(
        !read_by(0).conflicts(&moved(0)),
        "a thread is not its own sibling"
    );
    assert!(!read_by(0).conflicts(&read_by(1)));
    assert!(!moved(1).conflicts(&StepFootprint::default()));
}

#[test]
fn the_step_that_finishes_a_ppe_thread_pairs_with_the_other_ppe_thread_only() {
    let mut rt = Runtime::new(GuestMemory::new(0x100), Budget::new(1), 64);
    let busy = || vec![FakeOp::LoadImm(1); 8];
    let short = rt.register_unit_with(|id| {
        FakeIsaUnit::new(id, vec![FakeOp::LoadImm(1), FakeOp::End]).as_ppe_thread()
    });
    let sibling = rt.register_unit_with(|id| FakeIsaUnit::new(id, busy()).as_ppe_thread());
    let other = rt.register_unit_with(|id| FakeIsaUnit::new(id, busy()));

    let mut steps = Vec::new();
    for _ in 0..12 {
        let step = rt.step().expect("a runnable unit");
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
        let mut footprint = StepFootprint::from_effects(&step.effects);
        footprint.note_commit(&rt, step.unit);
        steps.push((step.unit, footprint));
    }

    let finishing: Vec<_> = steps
        .iter()
        .filter(|(_, f)| !f.ppe_status_changes.is_empty())
        .collect();
    assert_eq!(finishing.len(), 1, "one step moves a PPE thread: {steps:?}");
    let (unit, finish) = finishing[0];
    assert_eq!(*unit, short);
    assert_eq!(finish.ppe_status_changes, [short]);
    assert_eq!(finish.ppe_sibling_reader, Some(short));

    let step_of = |u: UnitId| {
        steps
            .iter()
            .find(|(s, _)| *s == u)
            .map(|(_, f)| f)
            .expect("the unit stepped")
    };
    assert_eq!(step_of(sibling).ppe_sibling_reader, Some(sibling));
    assert!(finish.conflicts(step_of(sibling)));
    assert!(step_of(sibling).conflicts(finish));
    assert_eq!(step_of(other).ppe_sibling_reader, None);
    assert!(!finish.conflicts(step_of(other)));
}

#[test]
fn a_step_that_wakes_a_parked_ppe_thread_records_the_wake_though_the_waker_is_no_ppe_thread() {
    let mut rt = Runtime::new(GuestMemory::new(0x100), Budget::new(1), 64);
    let parker = rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![FakeOp::Wait { signal: 0, mask: 1 }, FakeOp::LoadImm(1)],
        )
        .as_ppe_thread()
    });
    let sibling = rt
        .register_unit_with(|id| FakeIsaUnit::new(id, vec![FakeOp::LoadImm(1); 8]).as_ppe_thread());
    let waker = rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![FakeOp::Wake { unit: parker.raw() }, FakeOp::LoadImm(1)],
        )
    });

    let mut steps = Vec::new();
    for _ in 0..4 {
        let step = rt.step().expect("a runnable unit");
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
        let mut footprint = StepFootprint::from_effects(&step.effects);
        footprint.note_commit(&rt, step.unit);
        steps.push((step.unit, footprint));
    }

    let order: Vec<UnitId> = steps.iter().map(|(u, _)| *u).collect();
    assert_eq!(
        order[..3],
        [parker, sibling, waker],
        "the lowest clock runs first and a tie goes to the lower id",
    );
    let (_, park) = &steps[0];
    let (_, read) = &steps[1];
    let (_, wake) = &steps[2];
    assert_eq!(
        park.ppe_status_changes,
        [parker],
        "the park moves the parker"
    );
    assert_eq!(wake.ppe_sibling_reader, None);
    assert_eq!(
        wake.ppe_status_changes,
        [parker],
        "the commit of another unit's step moved the parker back",
    );
    assert!(wake.conflicts(read));
    assert!(read.conflicts(wake));
}
