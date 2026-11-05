//! A write that no unit commits as a store neither adds nor clears a
//! record of an unsynchronized code fetch. These writes are:
//!
//! - the loader's writes into the image before the first step;
//! - the host's placements during the run.

use std::collections::VecDeque;

use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_time::Budget;

use super::tests::{fetch, memory, step, store, Scripted, Step, NOP, X};
use crate::runtime::{AddressSpaceId, Runtime};

/// A runtime over [`memory`], whose `X` the loader wrote before the run,
/// with one scripted unit.
fn runtime(steps: &[Step]) -> Runtime {
    let mut rt = Runtime::new(memory(), Budget::new(1), 1000);
    let steps: VecDeque<Step> = steps.iter().cloned().collect();
    rt.register_unit_with(|id| Scripted {
        id,
        steps,
        drained: Vec::new(),
    });
    rt
}

fn run_step(rt: &mut Runtime) {
    let mut s = rt.step().expect("the unit is runnable");
    rt.commit_step(&mut s).expect("the batch commits");
}

#[test]
fn a_fetch_of_a_word_the_loader_or_a_placement_wrote_is_not_recorded() {
    let mut rt = runtime(&[step(&[]), step(&[fetch(X), fetch(0x1C4)])]);
    run_step(&mut rt);
    let range = ByteRange::new(GuestAddr::new(0x1C4), 4).unwrap();
    rt.place_bytes(AddressSpaceId::BOOT, range, &NOP.to_be_bytes())
        .expect("the placement lands");
    run_step(&mut rt);

    assert_eq!(rt.unsynced_code_fetches(), []);
}

#[test]
fn a_placement_over_a_guest_stored_line_keeps_its_record() {
    let mut rt = runtime(&[step(&[store(X, NOP)]), step(&[fetch(X)])]);
    run_step(&mut rt);
    let range = ByteRange::new(GuestAddr::new(X), 4).unwrap();
    rt.place_bytes(AddressSpaceId::BOOT, range, &NOP.to_be_bytes())
        .expect("the placement lands");
    run_step(&mut rt);

    let [record] = rt.unsynced_code_fetches() else {
        panic!("one record, got {:?}", rt.unsynced_code_fetches());
    };
    assert_eq!(record.store_step, 1, "the guest store's step");
}
