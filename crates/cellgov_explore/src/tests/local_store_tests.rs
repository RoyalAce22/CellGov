//! The local-store side of a transfer in the dependency relation: a
//! transfer reads or writes local store when it completes, so a step of
//! the unit that owns those bytes, taken while the transfer is in flight,
//! falls on one side of the landing or the other.

use crate::dependency::StepFootprint;
use cellgov_core::Runtime;
use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

fn range(start: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(start), len).unwrap()
}

/// A get of 16 bytes from main storage into `issuer`'s local store at
/// `lsa`.
fn get(issuer: UnitId, lsa: u64) -> Effect {
    Effect::DmaEnqueue {
        request: DmaRequest::new(DmaDirection::Get, range(0x80, 16), range(lsa, 16), issuer)
            .unwrap(),
        payload: None,
    }
}

#[test]
fn a_get_claims_its_issuer_local_store_and_a_put_from_local_store_claims_its_source() {
    let issuer = UnitId::new(3);
    let got = StepFootprint::from_effects(&[get(issuer, 0x400)]);
    assert_eq!(got.dma_local_stores, [(issuer, range(0x400, 16))]);

    let put = DmaRequest::new(DmaDirection::Put, range(0x500, 16), range(0x80, 16), issuer)
        .unwrap()
        .with_local_store_source();
    let put = StepFootprint::from_effects(&[Effect::DmaEnqueue {
        request: put,
        payload: None,
    }]);
    assert_eq!(put.dma_local_stores, [(issuer, range(0x500, 16))]);
    assert!(put.dma_reads.is_empty(), "the source is not main storage");
}

#[test]
fn an_enqueue_pairs_with_a_flight_into_the_same_local_store_bytes_only() {
    let owner = UnitId::new(3);
    let during = StepFootprint {
        inflight_local_stores: vec![(owner, range(0x400, 16))],
        ..StepFootprint::default()
    };
    assert!(during.conflicts(&StepFootprint::from_effects(&[get(owner, 0x408)])));
    assert!(!during.conflicts(&StepFootprint::from_effects(&[get(owner, 0x410)])));
    assert!(!during.conflicts(&StepFootprint::from_effects(&[get(UnitId::new(4), 0x400)])));
}

/// A footprint of a step `owner` took, touching nothing shared.
fn stepped(owner: u64) -> StepFootprint {
    StepFootprint {
        local_store_owner: Some(UnitId::new(owner)),
        ..StepFootprint::default()
    }
}

/// A footprint of a step `unit` took while a transfer into unit 3's
/// local store was in flight.
fn during_flight(unit: u64) -> StepFootprint {
    StepFootprint {
        inflight_local_stores: vec![(UnitId::new(3), range(0x400, 16))],
        ..stepped(unit)
    }
}

#[test]
fn a_step_of_the_owner_during_its_landing_conflicts_with_every_step() {
    let rides = during_flight(3);
    assert!(rides.conflicts(&StepFootprint::default()));
    assert!(StepFootprint::default().conflicts(&rides));
}

#[test]
fn a_step_of_the_owner_pairs_with_another_unit_step_during_its_landing() {
    // The landing can fire in the other unit's step, so the owner's later
    // loads fall on one side of it or the other.
    assert!(during_flight(5).conflicts(&stepped(3)));
    assert!(stepped(3).conflicts(&during_flight(5)));
    assert!(!during_flight(5).conflicts(&stepped(6)));
}

/// Step `unit` once and commit `effects` in place of what it emitted,
/// then return the footprint of that step.
fn step_with(rt: &mut Runtime, effects: &[Effect]) -> (UnitId, StepFootprint) {
    let step = rt.step().expect("a runnable unit");
    rt.commit_step(&step.result, effects)
        .expect("the step commits");
    let mut footprint = StepFootprint::from_effects(effects);
    footprint.note_commit(rt, step.unit);
    (step.unit, footprint)
}

#[test]
fn the_steps_after_the_enqueue_carry_the_flight_and_only_the_owner_rides_it() {
    let mut rt = Runtime::new(GuestMemory::new(0x100), Budget::new(1), 64);
    let ops = || vec![FakeOp::LoadImm(1), FakeOp::LoadImm(2), FakeOp::LoadImm(3)];
    let owner = rt.register_unit_with(|id| FakeIsaUnit::new(id, ops()));
    rt.register_unit_with(|id| FakeIsaUnit::new(id, ops()));

    let (unit, enqueued) = step_with(&mut rt, &[get(owner, 0x400)]);
    assert_eq!(unit, owner);
    assert!(
        enqueued.inflight_local_stores.is_empty(),
        "the enqueuing step's instructions all ran before the transfer was queued: {enqueued:?}"
    );

    let mut steps = Vec::new();
    for _ in 0..2 {
        steps.push(step_with(&mut rt, &[]));
    }
    assert!(
        rt.dma_queue().pending().next().is_some(),
        "the get is still in flight"
    );
    for (unit, footprint) in &steps {
        assert_eq!(footprint.inflight_local_stores, [(owner, range(0x400, 16))]);
        assert_eq!(
            footprint.conflicts(&StepFootprint::default()),
            *unit == owner,
            "only the owner rides the landing: {unit:?}"
        );
    }
    assert!(
        steps.iter().any(|(unit, _)| *unit == owner),
        "the owner stepped during the flight"
    );
}
