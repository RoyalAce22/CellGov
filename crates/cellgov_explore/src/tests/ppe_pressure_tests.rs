//! The PPE issue pressure in the dependency relation: a PPE thread's
//! instructions wait out the stalls of the sibling beside it, so a step
//! that changes a PPE thread's pressure orders against its sibling's.

use crate::dependency::StepFootprint;
use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_exec::PpeIssuePressure;
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

/// A divide sibling's pressure.
const DIVIDES: PpeIssuePressure = PpeIssuePressure {
    cycles: 13,
    vsu_cycles: 13,
    multiply_cycles: 13,
    multiply_hold_share: 256,
    issue_interval: 0,
};

/// A footprint of a step PPE thread `unit` took.
fn read_by(unit: u64) -> StepFootprint {
    StepFootprint {
        ppe_sibling_reader: Some(UnitId::new(unit)),
        ..StepFootprint::default()
    }
}

/// A footprint of a step that changed PPE thread `unit`'s pressure.
fn pressed_by(unit: u64) -> StepFootprint {
    StepFootprint {
        ppe_sibling_reader: Some(UnitId::new(unit)),
        ppe_pressure_changer: Some(UnitId::new(unit)),
        ..StepFootprint::default()
    }
}

#[test]
fn a_ppe_step_pairs_with_a_step_that_changes_another_ppe_thread_s_pressure_only() {
    assert!(read_by(0).conflicts(&pressed_by(1)));
    assert!(pressed_by(1).conflicts(&read_by(0)));
    assert!(
        !read_by(0).conflicts(&StepFootprint {
            ppe_pressure_changer: Some(UnitId::new(0)),
            ..StepFootprint::default()
        }),
        "a thread does not wait out its own stalls through a sibling"
    );
    assert!(!pressed_by(1).conflicts(&StepFootprint::default()));
}

#[test]
fn a_restore_forgets_the_last_step_s_pressure() {
    let mut rt = Runtime::new(GuestMemory::new(0x100), Budget::new(1), 64);
    rt.register_unit_with(|id| FakeIsaUnit::new(id, vec![FakeOp::LoadImm(1); 4]).as_ppe_thread());
    let snap = rt.snapshot();
    let step = rt.step().expect("a runnable unit");
    rt.commit_step(&step.result, &step.effects)
        .expect("the step commits");
    assert!(rt.last_ppe_pressure().is_some());
    rt.restore_into(&snap);
    assert_eq!(rt.last_ppe_pressure(), None);
}

#[test]
fn only_the_step_that_changes_a_ppe_thread_s_pressure_records_it() {
    let mut rt = Runtime::new(GuestMemory::new(0x100), Budget::new(1), 64);
    let presser = rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::LoadImm(1),
                FakeOp::SetPpePressure(DIVIDES),
                FakeOp::SetPpePressure(DIVIDES),
                FakeOp::LoadImm(1),
            ],
        )
        .as_ppe_thread()
    });
    let sibling = rt
        .register_unit_with(|id| FakeIsaUnit::new(id, vec![FakeOp::LoadImm(1); 8]).as_ppe_thread());

    let mut pressing = Vec::new();
    let mut reads = Vec::new();
    for _ in 0..8 {
        let step = rt.step().expect("a runnable unit");
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
        let mut footprint = StepFootprint::from_effects(&step.effects);
        footprint.note_commit(&rt, step.unit);
        if step.unit == presser {
            pressing.push(footprint.ppe_pressure_changer);
        } else {
            reads.push(footprint);
        }
    }
    assert_eq!(
        pressing,
        [None, Some(presser), None, None],
        "the step that sets a divide's pressure changes it, the one that sets it again does not"
    );
    let change = StepFootprint {
        ppe_pressure_changer: Some(presser),
        ..StepFootprint::default()
    };
    assert_eq!(reads.len(), 4);
    for read in &reads {
        assert_eq!(read.ppe_sibling_reader, Some(sibling));
        assert!(read.conflicts(&change));
    }
}
