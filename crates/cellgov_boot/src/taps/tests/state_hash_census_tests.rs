//! The state-hash census's counts and samples, and the observer that
//! `WithPpuTap` builds.

use std::cell::Cell;

use cellgov_event::UnitId;
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::state::PpuState;

use super::*;
use crate::NoTaps;

fn dispatch(tap: &dyn PpuTap, state: &PpuState) {
    tap.dispatch(UnitId::new(0), &PpuInstruction::Consumed, state);
}

fn with_gpr(k: usize, v: u64) -> PpuState {
    let mut s = PpuState::new();
    s.set_gpr(k, v);
    s
}

#[test]
fn a_repeated_state_counts_once() {
    let census = StateHashCensus::new(0);
    let a = with_gpr(3, 1);
    let b = with_gpr(3, 2);
    for s in [&a, &b, &a, &a, &b] {
        dispatch(&census, s);
    }
    let report = census.report();
    assert_eq!(report.dispatches, 5);
    assert_eq!(report.distinct_states, 2);
    assert_eq!(report.state_hash_collisions, 0);
    assert_eq!(report.multilinear_collisions, 0);
    assert_eq!(report.identity_conflicts, 0);
    assert!(report.samples.is_empty());
}

#[test]
fn distinct_states_that_share_a_hash_count_as_collisions() {
    let hashes = [7, 7, 9, 7, 9];
    assert_eq!(collisions(hashes.to_vec()), 3);
    assert_eq!(collisions(vec![1, 2, 3]), 0);
}

#[test]
fn a_state_the_census_saw_twice_with_two_hashes_is_an_identity_conflict() {
    let census = StateHashCensus::new(0);
    dispatch(&census, &with_gpr(1, 1));
    // A second record under the same identity with another hash is what
    // an identity collision leaves.
    let mut forged = census.records.borrow()[0];
    forged.state_hash ^= 1;
    census.records.borrow_mut().push(forged);
    let report = census.report();
    assert_eq!(report.distinct_states, 1);
    assert_eq!(report.identity_conflicts, 1);
}

#[test]
fn each_shared_hash_is_counted_against_its_own_function() {
    let census = StateHashCensus::new(0);
    for v in 1..=3 {
        dispatch(&census, &with_gpr(1, v));
    }
    // Three states share one state_hash; two of them share one
    // multilinear hash.
    {
        let mut records = census.records.borrow_mut();
        let first = records[0];
        records[1].state_hash = first.state_hash;
        records[2].state_hash = first.state_hash;
        records[1].multilinear = first.multilinear;
    }
    let report = census.report();
    assert_eq!(report.distinct_states, 3);
    assert_eq!(report.state_hash_collisions, 2);
    assert_eq!(report.multilinear_collisions, 1);
    assert_eq!(report.identity_conflicts, 0);
}

#[test]
fn samples_are_the_lanes_at_each_interval() {
    let census = StateHashCensus::new(2);
    for v in 0..5 {
        dispatch(&census, &with_gpr(0, v));
    }
    let samples = census.report().samples;
    let got: Vec<(u64, u64)> = samples.iter().map(|(i, l)| (*i, l[0])).collect();
    assert_eq!(got, [(0, 0), (2, 2), (4, 4)]);
}

/// A PPU observer that counts its dispatches.
#[derive(Default)]
struct Counter(Cell<u32>);

impl PpuTap for Counter {
    fn dispatch(&self, _: UnitId, _: &PpuInstruction, _: &PpuState) {
        self.0.set(self.0.get() + 1);
    }
}

/// Debug taps whose only observer is a PPU counter.
struct CounterTaps(Rc<Counter>);

impl DebugTaps for CounterTaps {
    fn ppu(&self) -> Option<Rc<dyn PpuTap>> {
        Some(Rc::clone(&self.0) as Rc<dyn PpuTap>)
    }
}

#[test]
fn the_extra_observer_sees_each_dispatch_beside_the_inner_one() {
    let inner = Rc::new(Counter::default());
    let census = Rc::new(StateHashCensus::new(0));
    let taps = WithPpuTap::new(
        Rc::new(CounterTaps(Rc::clone(&inner))),
        Rc::clone(&census) as Rc<dyn PpuTap>,
    );
    let ppu = taps.ppu().expect("a PPU observer");
    dispatch(ppu.as_ref(), &PpuState::new());
    dispatch(ppu.as_ref(), &PpuState::new());
    assert_eq!(inner.0.get(), 2);
    assert_eq!(census.report().dispatches, 2);
}

/// Debug taps that count the runtime and firmware calls they get.
#[derive(Default)]
struct AskedTaps {
    runtimes: Cell<u32>,
    bound: Cell<u32>,
}

impl DebugTaps for AskedTaps {
    fn runtime(&self) -> Option<Box<dyn cellgov_core::RuntimeTap>> {
        self.runtimes.set(self.runtimes.get() + 1);
        None
    }

    fn firmware_bound(
        &self,
        _: u32,
        _: &std::collections::BTreeMap<String, std::collections::BTreeMap<u32, u32>>,
        _: &cellgov_mem::GuestMemory,
    ) {
        self.bound.set(self.bound.get() + 1);
    }
}

#[test]
fn the_runtime_and_firmware_calls_reach_the_inner_taps() {
    let inner = Rc::new(AskedTaps::default());
    let taps = WithPpuTap::new(
        Rc::clone(&inner) as Rc<dyn DebugTaps>,
        Rc::new(StateHashCensus::new(0)) as Rc<dyn PpuTap>,
    );
    assert!(taps.runtime().is_none());
    taps.firmware_bound(
        0,
        &std::collections::BTreeMap::new(),
        &cellgov_mem::GuestMemory::new(0x1000),
    );
    assert_eq!(inner.runtimes.get(), 1);
    assert_eq!(inner.bound.get(), 1);
}

#[test]
fn with_no_inner_observer_the_extra_one_is_the_observer() {
    let census = Rc::new(StateHashCensus::new(0));
    let taps = WithPpuTap::new(Rc::new(NoTaps), Rc::clone(&census) as Rc<dyn PpuTap>);
    dispatch(
        taps.ppu().expect("a PPU observer").as_ref(),
        &PpuState::new(),
    );
    assert_eq!(census.report().dispatches, 1);
    assert!(taps.runtime().is_none());
}

#[test]
fn each_identity_key_set_meets_the_key_checks_of_the_construction() {
    let mask = (1u128 << 65) - 1;
    for seed in IDENTITY_SEEDS {
        let keys = multilinear::derive_keys(seed);
        assert_ne!(keys, multilinear::KEYS, "seed {seed}");
        let mults = &keys[1..];
        for (i, &a) in mults.iter().enumerate() {
            for (j, &b) in mults.iter().enumerate() {
                assert_ne!(a.wrapping_add(b) & mask, 0, "seed {seed}: lanes {i}, {j}");
                if i != j {
                    assert_ne!(a.wrapping_sub(b) & mask, 0, "seed {seed}: lanes {i}, {j}");
                }
            }
        }
    }
}
