use std::collections::BTreeMap;

use cellgov_ppu::differential::{execute_state_in, PpuStateSnapshot};
use cellgov_ps3_abi::hw::ppc_isa::PPC_FPSCR_OE;
use cellgov_ps3_abi::hw::ppu_status::{FPSCR_FR, FPSCR_OX};

use super::*;
use crate::ppu_documented::replay::{initial_state, MEM_BYTES, REPLAY_MEM_BASE};
use crate::ppu_documented::vector::{Component, DocumentedVector, Location, Verdict};
use crate::ppu_documented::{documented_vectors, Set};

/// The state before `vector`'s last word and the state after it.
fn around_last_word(vector: &DocumentedVector) -> (PpuStateSnapshot, PpuStateSnapshot) {
    let mut state = initial_state(vector, REPLAY_MEM_BASE);
    let mut mem = vector.mem.clone().unwrap_or_default();
    mem.resize(MEM_BYTES, 0);
    let (last, prefix) = vector.words.split_last().expect("a vector runs a word");
    for &word in prefix {
        (state, mem) = execute_state_in(word, &state, REPLAY_MEM_BASE, mem).expect("prefix runs");
    }
    let (after, _) =
        execute_state_in(*last, &state, REPLAY_MEM_BASE, mem).expect("the last word runs");
    (state, after)
}

/// Whether `places` hold every bit of `component`.
fn covers(places: &[OpenPlace], component: &Component) -> bool {
    let mask = component.mask;
    let held = |test: &dyn Fn(OpenAt) -> u128| places.iter().fold(0u128, |acc, p| acc | test(p.at));
    match component.location {
        Location::Gpr(n) => {
            let open = held(&|at| match at {
                OpenAt::Gpr { index, mask } if index == n => u128::from(mask),
                _ => 0,
            });
            mask & !open == 0
        }
        Location::Fpr(n) => {
            let open = held(&|at| match at {
                OpenAt::Fpr { index, mask } if index == n => u128::from(mask),
                _ => 0,
            });
            mask & !open == 0
        }
        Location::Vr(n) => {
            let open = held(&|at| match at {
                OpenAt::Vr { index, mask } if index == n => mask,
                _ => 0,
            });
            mask & !open == 0
        }
        Location::Cr => {
            mask & !held(&|at| match at {
                OpenAt::Cr(bits) => u128::from(bits),
                _ => 0,
            }) == 0
        }
        Location::Xer => {
            mask & !held(&|at| match at {
                OpenAt::Xer(bits) => u128::from(bits),
                _ => 0,
            }) == 0
        }
        Location::Fpscr => {
            mask & !held(&|at| match at {
                OpenAt::Fpscr(bits) => u128::from(bits),
                _ => 0,
            }) == 0
        }
        Location::Mem(off) => {
            let start = REPLAY_MEM_BASE + u64::from(off);
            (0..8u64)
                .filter(|byte| mask >> (8 * (7 - byte)) & 0xff != 0)
                .all(|byte| {
                    places.iter().any(|p| match p.at {
                        OpenAt::Memory { ea, len } => (ea..ea + len).contains(&(start + byte)),
                        _ => false,
                    })
                })
        }
    }
}

/// Whether `places` hold any bit of `component`.
fn touches(places: &[OpenPlace], component: &Component) -> bool {
    let bits = |c: &Component| -> Vec<Component> {
        (0..128)
            .filter(|bit| c.mask >> bit & 1 == 1)
            .map(|bit| Component {
                mask: 1 << bit,
                ..c.clone()
            })
            .collect()
    };
    bits(component).iter().any(|bit| covers(places, bit))
}

/// Whether `vector` stores conditionally to another word of the reserved
/// line. The record names only the reserved line, so it cannot show this
/// open place.
fn same_line_store(vector: &DocumentedVector) -> bool {
    vector.set == Set::StwcxOtherAddress && vector.id.contains("/same-granule/")
}

#[test]
fn every_undefined_component_of_a_single_step_set_is_open_and_no_fixed_one_is() {
    let mut checked: BTreeMap<Set, usize> = BTreeMap::new();
    for vector in documented_vectors() {
        if vector.set == Set::ReservationUndefined || same_line_store(&vector) {
            continue;
        }
        let (pre, post) = around_last_word(&vector);
        let word = *vector.words.last().expect("a vector runs a word");
        // The trace replay reads the open places of every set, so the
        // test holds a fixed component against all of them.
        let all = open_places(word, &pre, [&post, &post]);
        let places: Vec<OpenPlace> = all
            .iter()
            .copied()
            .filter(|place| place.set == vector.set)
            .collect();
        let pages: Vec<&str> = vector
            .expected
            .iter()
            .map(|c| match c.verdict {
                Verdict::Undefined { citation } | Verdict::Value { citation, .. } => citation,
            })
            .collect();
        for place in &places {
            assert!(
                pages.contains(&place.citation),
                "{} opens {:?} under a page the vector does not cite",
                vector.id,
                place.at
            );
        }
        for component in &vector.expected {
            match component.verdict {
                Verdict::Undefined { .. } => assert!(
                    covers(&places, component),
                    "{} {} is not open: {places:?}",
                    vector.id,
                    component.name
                ),
                Verdict::Value { .. } => assert!(
                    !touches(&all, component),
                    "{} {} is fixed but open",
                    vector.id,
                    component.name
                ),
            }
        }
        *checked.entry(vector.set).or_default() += 1;
    }
    let sets: Vec<Set> = checked.keys().copied().collect();
    let expected: Vec<Set> = Set::ALL
        .into_iter()
        .filter(|&set| set != Set::ReservationUndefined)
        .collect();
    assert_eq!(sets, expected, "every single-step set is checked");
}

#[test]
fn a_dcbf_opens_the_reservation_only_when_it_names_the_reserved_line() {
    let mut cases = 0;
    for vector in documented_vectors() {
        if vector.set != Set::ReservationUndefined {
            continue;
        }
        let mut state = initial_state(&vector, REPLAY_MEM_BASE);
        let mut mem = vector.mem.clone().unwrap_or_default();
        mem.resize(MEM_BYTES, 0);
        (state, mem) =
            execute_state_in(vector.words[0], &state, REPLAY_MEM_BASE, mem).expect("reserve runs");
        let (after, _) =
            execute_state_in(vector.words[1], &state, REPLAY_MEM_BASE, mem).expect("dcbf runs");
        let places = open_places(vector.words[1], &state, [&after, &after]);
        let open = places
            .iter()
            .any(|p| p.set == Set::ReservationUndefined && p.at == OpenAt::Reservation);
        let undefined = vector
            .expected
            .iter()
            .any(|c| matches!(c.verdict, Verdict::Undefined { .. }));
        assert_eq!(open, undefined, "{}", vector.id);
        cases += 1;
    }
    assert_eq!(cases, 4);
}

#[test]
fn a_store_conditional_to_another_word_of_the_reserved_line_stays_closed() {
    let vectors: Vec<DocumentedVector> = documented_vectors()
        .into_iter()
        .filter(same_line_store)
        .collect();
    assert_eq!(vectors.len(), 4);
    for vector in vectors {
        let (pre, post) = around_last_word(&vector);
        let word = *vector.words.last().expect("a vector runs a word");
        assert_eq!(
            open_places(word, &pre, [&post, &post]),
            Vec::new(),
            "{}",
            vector.id
        );
    }
}

#[test]
fn an_overflow_opens_fr_only_when_a_side_newly_sets_ox_with_oe_clear() {
    // fmul f3, f1, f2.
    let word = 0xfc61_00b2;
    let pre = PpuStateSnapshot::zero();
    let mut overflowed = pre.clone();
    overflowed.fpscr = FPSCR_OX;
    let fr = |places: Vec<OpenPlace>| places.iter().any(|p| p.at == OpenAt::Fpscr(FPSCR_FR));
    assert!(fr(open_places(word, &pre, [&pre, &overflowed])));
    assert!(!fr(open_places(word, &pre, [&pre, &pre])));
    let mut enabled = pre.clone();
    enabled.fpscr = PPC_FPSCR_OE;
    assert!(!fr(open_places(word, &enabled, [&enabled, &overflowed])));
    let mut sticky = pre.clone();
    sticky.fpscr = FPSCR_OX;
    assert!(!fr(open_places(word, &sticky, [&overflowed, &overflowed])));
}

#[test]
fn a_word_the_decoder_refuses_opens_nothing() {
    let pre = PpuStateSnapshot::zero();
    assert_eq!(open_places(0, &pre, [&pre, &pre]), Vec::new());
}
