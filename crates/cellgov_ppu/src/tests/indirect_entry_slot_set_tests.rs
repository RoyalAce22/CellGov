use std::collections::BTreeSet;

use super::*;

/// A xorshift generator, so each run makes the same operations.
struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % bound
    }

    /// An offset in `-80..80`, which can lie out of [`REACH`].
    fn offset(&mut self) -> i64 {
        i64::try_from(self.below(160)).expect("small") - 80
    }

    fn slot(&mut self) -> Slot {
        let width = [1, 2, 4, 8][usize::try_from(self.below(4)).expect("small")];
        (self.offset(), width)
    }

    fn slots(&mut self, most: u64) -> BTreeSet<Slot> {
        (0..self.below(most + 1)).map(|_| self.slot()).collect()
    }
}

const REACH: Range<i64> = -64..64;

/// The moves of r1, multiples of 16 and others.
const MOVES: [i64; 8] = [-48, -16, -8, -4, 4, 8, 16, 48];

fn overlapping(slots: &BTreeSet<Slot>, offset: i64, end: i64) -> bool {
    slots
        .iter()
        .any(|&(held, width)| held < end && offset < held + i64::from(width))
}

/// The number of runs that a set of `slots` keeps. A slot starts a run
/// when `slots` holds no slot of its width 16 bytes below it.
fn run_starts(slots: &BTreeSet<Slot>) -> usize {
    slots
        .iter()
        .filter(|&&(offset, width)| !slots.contains(&(offset - 16, width)))
        .count()
}

#[test]
fn each_operation_of_a_slot_set_matches_the_same_operation_on_its_slots_and_runs() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut set = SlotSet::default();
    let mut model = BTreeSet::new();
    let (mut grown, mut unchanged, mut merged) = (0, 0, 0);
    for _ in 0..4000 {
        match rng.below(5) {
            0 | 1 => {
                let slot = rng.slot();
                set.insert(slot);
                model.insert(slot);
            }
            2 => {
                let offset = rng.offset();
                let end = offset + i64::try_from(rng.below(24)).expect("small");
                set = set.without_inside(offset, end);
                model.retain(|&(held, width)| held < offset || end < held + i64::from(width));
            }
            3 => {
                let by = MOVES[usize::try_from(rng.below(8)).expect("small")];
                set = set.moved(by, &REACH);
                model = model
                    .iter()
                    .filter_map(|&(offset, width)| {
                        let offset = offset.checked_sub(by)?;
                        REACH.contains(&offset).then_some((offset, width))
                    })
                    .collect();
            }
            _ => {
                let other = rng.slots(6);
                let grew = !other.is_subset(&model);
                if grew {
                    grown += 1;
                } else {
                    unchanged += 1;
                }
                assert_eq!(set.join(&SlotSet::of(other.iter().copied())), grew);
                model.extend(other);
            }
        }
        assert_eq!(set.slots().collect::<BTreeSet<_>>(), model);
        assert_eq!(set.len(), model.len());
        let runs = run_starts(&model);
        assert_eq!(set.run_count(), runs);
        if runs < model.len() {
            merged += 1;
        }
        assert_eq!(set, SlotSet::of(model.iter().copied()));
        let offset = rng.offset();
        let end = offset + 1 + i64::try_from(rng.below(12)).expect("small");
        assert_eq!(set.overlaps(offset, end), overlapping(&model, offset, end));
        let slot = rng.slot();
        assert_eq!(set.contains(slot), model.contains(&slot));
        let other = rng.slots(3);
        let part: BTreeSet<Slot> = model.iter().copied().step_by(2).chain(other).collect();
        assert_eq!(
            SlotSet::of(part.iter().copied()).is_subset(&set),
            part.is_subset(&model)
        );
    }
    assert!(grown >= 100, "{grown} joins grew the set");
    assert!(unchanged >= 20, "{unchanged} joins left the set as it was");
    assert!(
        merged >= 1000,
        "{merged} steps held a run of more than one slot"
    );
}

#[test]
fn a_join_of_a_superset_into_a_set_shares_the_runs_of_the_superset() {
    let mut set = SlotSet::of([(8, 8), (-8, 8)]);
    let superset = SlotSet::of([(8, 8), (-8, 8), (-24, 8), (4, 4)]);
    assert!(set.join(&superset));
    assert!(set.shares(&superset));
    assert!(!set.join(&superset));
}
