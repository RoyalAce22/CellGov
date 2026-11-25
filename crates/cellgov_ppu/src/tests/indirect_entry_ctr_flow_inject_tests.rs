use cellgov_ps3_abi::hw::ppc_isa::PPC_NOP;

use super::*;

const BASE: u64 = 0x8000_0000_0010_0000;

/// The state after `inject` joins `copies` into the word whose state is
/// `state`, and whether it queued the word.
fn injected(state: &CtrState, copies: &CtrState, before: &SlotSet) -> (CtrState, bool) {
    let code = Code::new(vec![CodeSegment {
        vaddr: BASE,
        words: vec![PPC_NOP],
    }]);
    let (writes, lost_writes) = (BTreeMap::new(), BTreeSet::new());
    let reach = Reach::every(&code);
    let mut flow = CtrFlow::new(&code, &writes, &lost_writes, &reach);
    flow.at[0][0] = state.clone();
    let mut work = Worklist::new(&code, &reach);
    assert_eq!(work.pop(), Some((0, 0)));
    assert_eq!(work.pop(), None);
    flow.inject((0, 0), copies, before, &mut work);
    (flow.at[0][0].clone(), work.pop().is_some())
}

/// The state after a join of `copies` into `state`, and whether the join
/// changed it.
fn joined(state: &CtrState, copies: &CtrState) -> (CtrState, bool) {
    let mut joined = state.clone();
    let changed = joined.join(copies);
    (joined, changed)
}

#[test]
fn an_injection_gives_the_state_and_queue_of_a_join_with_or_without_shared_slots() {
    let before = SlotSet::of([(8, 8), (-8, 8)]);
    let mut grown = before.clone();
    assert!(grown.join(&SlotSet::of([(-24, 8)])));
    let holding = |slots: &SlotSet| CtrState::lost_copies(1 << 3, slots);
    let unshared = SlotSet::of([(8, 8), (-8, 8)]);
    assert!(!unshared.shares(&before));
    let wider = SlotSet::of([(8, 8), (-8, 8), (-40, 8)]);
    assert!(before.is_subset(&wider) && !wider.is_subset(&before));
    let cases = [
        (
            holding(&before),
            CtrState::lost_copies(1 << 3 | 1 << 5, &before),
        ),
        (holding(&before), CtrState::lost_copies(1 << 3, &grown)),
        (holding(&before), CtrState::lost_copies(1 << 3, &before)),
        (holding(&unshared), CtrState::lost_copies(1 << 3, &grown)),
        (holding(&unshared), CtrState::lost_copies(1 << 3, &before)),
        (holding(&wider), CtrState::lost_copies(1 << 3, &before)),
    ];
    let queued: Vec<bool> = cases
        .iter()
        .map(|(state, copies)| {
            let (after, queued) = injected(state, copies, &before);
            assert_eq!((after, queued), joined(state, copies));
            queued
        })
        .collect();
    assert_eq!(queued, [true, true, false, true, false, false]);
}

/// [PPC-Book1 p:42 s:3.3.3] `stw` stores RS 32:63 into the word at EA.
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
#[test]
fn a_store_over_part_of_a_lost_slot_keeps_it_lost_and_a_store_over_all_of_it_removes_only_it() {
    let neighbours = [(8, 8), (24, 8)];
    let mut slots = SlotCopies {
        known: BTreeMap::new(),
        lost: SlotSet::of([(8, 8), (16, 8), (24, 8)]),
    };
    slots.store((20, 4), None);
    assert_eq!(slots.lost, SlotSet::of([(8, 8), (16, 8), (24, 8)]));
    assert!(slots.loaded((16, 8), Load::Zero).lost);
    assert!(slots.loaded((20, 4), Load::Zero).lost);
    slots.store((16, 8), None);
    assert_eq!(slots.lost, SlotSet::of(neighbours));
    assert_eq!(slots.loaded((16, 8), Load::Zero), Ctr::default());
}

/// [PPC-Book1 p:42 s:3.3.3] `stw` stores RS 32:63 into the word at EA.
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
#[test]
fn a_known_slot_survives_a_store_beside_it_and_a_store_over_part_of_it_makes_it_lost() {
    let copy = Ctr {
        known: BTreeMap::from([(BASE, Value::sorted(vec![0x1234_5678_9ABC_DEF0]))]),
        lost: false,
    };
    let stored = || {
        let mut slots = SlotCopies {
            known: BTreeMap::new(),
            lost: SlotSet::default(),
        };
        slots.store((16, 8), Some(&copy));
        slots
    };
    let fresh = stored();
    assert_eq!(fresh.loaded((16, 8), Load::Zero), copy);
    assert!(fresh.lost.is_empty());
    for beside in [(8, 8), (24, 8), (12, 4), (24, 4)] {
        let mut slots = stored();
        slots.store(beside, None);
        assert_eq!(slots.loaded((16, 8), Load::Zero), copy, "{beside:?}");
        assert!(slots.lost.is_empty(), "{beside:?}");
    }
    for part in [(16, 4), (20, 4), (12, 8), (20, 8)] {
        let mut slots = stored();
        slots.store(part, None);
        assert!(slots.known.is_empty(), "{part:?}");
        assert_eq!(slots.lost, SlotSet::of([(16, 8)]), "{part:?}");
    }
    let mut slots = stored();
    slots.store((16, 8), None);
    assert!(slots.known.is_empty());
    assert!(slots.lost.is_empty());
}
