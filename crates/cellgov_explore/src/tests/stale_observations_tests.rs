//! A footprint reads the runtime's observations only for the step they
//! belong to.

use crate::dependency::{StaleObservations, StepFootprint};
use cellgov_core::Runtime;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

#[test]
fn a_footprint_refuses_the_observations_of_a_later_step() {
    let mut rt = Runtime::new(GuestMemory::new(0x100), Budget::new(1), 64);
    rt.register_unit_with(|id| FakeIsaUnit::new(id, vec![FakeOp::LoadImm(1); 4]));

    let mut first = rt.step().expect("a runnable unit");
    let first_runnable = first.runnable.clone();
    let mut footprint = StepFootprint::from_effects(&first.effects);
    rt.commit_step(&mut first).expect("the step commits");
    let mut second = rt.step().expect("a runnable unit");
    assert_ne!(first.token.index(), second.token.index());

    let untouched = format!("{footprint:?}");
    assert_eq!(
        footprint.note_commit(&rt, &first, &first_runnable),
        Err(StaleObservations {
            observed: second.token.index(),
            committed: first.token.index(),
        })
    );
    assert_eq!(format!("{footprint:?}"), untouched);

    let second_runnable = second.runnable.clone();
    let mut footprint = StepFootprint::from_effects(&second.effects);
    rt.commit_step(&mut second).expect("the step commits");
    footprint
        .note_commit(&rt, &second, &second_runnable)
        .expect("the observations are the step's");
}
