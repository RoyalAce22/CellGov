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

    let first = rt.step().expect("a runnable unit");
    rt.commit_unchecked(&first.result, &first.effects)
        .expect("the step commits");
    let second = rt.step().expect("a runnable unit");
    assert_ne!(first.token.index(), second.token.index());

    let mut footprint = StepFootprint::from_effects(&first.effects);
    let untouched = format!("{footprint:?}");
    assert_eq!(
        footprint.note_commit(&rt, &first, &first.runnable),
        Err(StaleObservations {
            observed: second.token.index(),
            committed: first.token.index(),
        })
    );
    assert_eq!(format!("{footprint:?}"), untouched);

    rt.commit_unchecked(&second.result, &second.effects)
        .expect("the step commits");
    let mut footprint = StepFootprint::from_effects(&second.effects);
    footprint
        .note_commit(&rt, &second, &second.runnable)
        .expect("the observations are the step's");
}
