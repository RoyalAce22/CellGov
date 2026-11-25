//! Clones of one shadow: the shared build-time decode, the shared written
//! flags, and the decodes each clone refreshes alone.

use proptest::prelude::*;

use super::proptests::{build, op, program, Op, BASE};
use super::PredecodedShadow;
use crate::decode;
use crate::instruction::PpuInstruction;
use crate::shadow::quicken::quicken_insn;
use crate::shadow::test_support::{build_from_words, cmpwi_raw, li_raw, lwz_raw};

/// A model of a shadow with no sharing: one decode and one stale flag per
/// slot, which `invalidate_range` and `refresh` change in place.
struct Unshared {
    slots: Vec<Option<PpuInstruction>>,
    stale: Vec<bool>,
}

impl Unshared {
    fn of(shadow: &PredecodedShadow) -> Self {
        let slots = (0..shadow.len())
            .map(|i| shadow.slot(i).expect("index in range").0)
            .collect();
        Self {
            slots,
            stale: vec![false; shadow.len()],
        }
    }

    fn get(&self, idx: usize) -> Option<PpuInstruction> {
        if self.stale[idx] {
            None
        } else {
            self.slots[idx]
        }
    }

    fn consumed(&self, idx: usize) -> bool {
        matches!(self.slots[idx], Some(PpuInstruction::Consumed))
    }

    fn invalidate_range(&mut self, addr: u64, len: u64) {
        let end = addr.saturating_add(len);
        let shadow_end = BASE + 4 * self.slots.len() as u64;
        if len == 0 || addr >= shadow_end || end <= BASE {
            return;
        }
        let mut first = ((addr.max(BASE) - BASE) / 4) as usize;
        let mut last = ((end.min(shadow_end) - BASE) as usize)
            .div_ceil(4)
            .min(self.slots.len());
        if first > 0 && self.consumed(first) {
            first -= 1;
        }
        if last < self.slots.len() && self.consumed(last) {
            last += 1;
        }
        self.stale[first..last].fill(true);
    }

    fn refresh(&mut self, idx: usize, raw: u32) {
        let head = |i: usize| self.slots[i].is_some_and(|insn| insn.is_super_pair());
        if head(idx) && idx + 1 < self.slots.len() && self.consumed(idx + 1) {
            self.stale[idx + 1] = true;
        } else if self.consumed(idx) && idx > 0 && head(idx - 1) {
            self.stale[idx - 1] = true;
        }
        self.slots[idx] = decode::decode(raw)
            .ok()
            .map(|insn| quicken_insn(insn).unwrap_or(insn));
        self.stale[idx] = false;
    }

    fn apply(&mut self, op: Op) {
        match op {
            Op::Invalidate { offset, len } => {
                self.invalidate_range((BASE as i64 + offset) as u64, len);
            }
            Op::Refresh { slot, raw } if slot < self.slots.len() => self.refresh(slot, raw),
            Op::Refresh { .. } => {}
        }
    }
}

fn apply(shadow: &mut PredecodedShadow, op: Op) {
    match op {
        Op::Invalidate { offset, len } => {
            shadow.invalidate_range((BASE as i64 + offset) as u64, len);
        }
        Op::Refresh { slot, raw } => {
            shadow.refresh(BASE + 4 * slot as u64, raw);
        }
    }
}

#[test]
fn a_clone_shares_the_build_time_decode() {
    let words = [li_raw(3, 1), li_raw(4, 2)];
    let shadow = build_from_words(0, &words);
    assert!(shadow.clone().shares_decode(&shadow));
    assert!(!build_from_words(0, &words).shares_decode(&shadow));
}

#[test]
fn a_refresh_in_a_clone_never_reaches_its_source() {
    let source = build_from_words(0, &[li_raw(3, 1), li_raw(4, 2)]);
    let mut clone = source.clone();
    clone.invalidate_range(0, 4);
    clone.refresh(0, li_raw(3, 9));
    assert_eq!(clone.get(0), Some(PpuInstruction::Li { rt: 3, imm: 9 }));
    assert_eq!(source.get(0), None);
    assert_eq!(source.get(4), Some(PpuInstruction::Li { rt: 4, imm: 2 }));
}

#[test]
fn an_invalidation_in_one_clone_stales_the_slot_in_another() {
    let mut first = build_from_words(0, &[li_raw(3, 1), li_raw(4, 2)]);
    let second = first.clone();
    first.invalidate_range(4, 4);
    assert_eq!(second.get(4), None);
    assert_eq!(second.get(0), Some(PpuInstruction::Li { rt: 3, imm: 1 }));
}

#[test]
fn a_clone_keeps_its_refresh_until_it_gets_an_invalidation() {
    let mut first = build_from_words(0, &[li_raw(3, 1)]);
    let mut second = first.clone();
    second.invalidate_range(0, 4);
    second.refresh(0, li_raw(3, 5));
    first.invalidate_range(0, 4);
    assert_eq!(second.get(0), Some(PpuInstruction::Li { rt: 3, imm: 5 }));
    second.invalidate_range(0, 4);
    assert_eq!(second.get(0), None);
}

#[test]
fn a_refresh_of_a_fused_head_in_one_clone_stales_its_partner_in_every_clone() {
    let words = [lwz_raw(3, 1, 8), cmpwi_raw(0, 3, 42), li_raw(5, 7)];
    let mut first = build_from_words(0, &words);
    let second = first.clone();
    assert!(first.get(0).is_some_and(|head| head.is_super_pair()));
    assert_eq!(second.get(4), Some(PpuInstruction::Consumed));
    first.refresh(0, li_raw(3, 1));
    assert_eq!(first.get(0), Some(PpuInstruction::Li { rt: 3, imm: 1 }));
    assert_eq!(first.get(4), None, "a fresh Consumed would skip slot 1");
    assert_eq!(second.get(0), None);
    assert_eq!(second.get(4), None);
    assert_eq!(second.get(8), Some(PpuInstruction::Li { rt: 5, imm: 7 }));
}

proptest! {
    #[test]
    fn one_shadow_reads_as_the_unshared_shadow(
        words in program(),
        ops in proptest::collection::vec(op(48), 1..24),
    ) {
        let mut shadow = build(&words);
        let mut unshared = Unshared::of(&shadow);
        for op in ops {
            apply(&mut shadow, op);
            unshared.apply(op);
            for i in 0..shadow.len() {
                let expected = (unshared.slots[i], unshared.stale[i]);
                prop_assert_eq!(shadow.slot(i), Some(expected), "slot {} after {:?}", i, op);
                prop_assert_eq!(shadow.get(BASE + 4 * i as u64), unshared.get(i), "slot {}", i);
            }
        }
    }

    #[test]
    fn a_clone_reads_its_own_unshared_decode_or_misses(
        words in program(),
        ops in proptest::collection::vec((any::<bool>(), op(48)), 1..32),
    ) {
        let mut shadows = [build(&words), build(&words)];
        shadows[1] = shadows[0].clone();
        let mut unshared = [Unshared::of(&shadows[0]), Unshared::of(&shadows[1])];
        for (second, op) in ops {
            let at = usize::from(second);
            apply(&mut shadows[at], op);
            unshared[at].apply(op);
            for (shadow, unshared) in shadows.iter().zip(&unshared) {
                for i in 0..shadow.len() {
                    let got = shadow.get(BASE + 4 * i as u64);
                    prop_assert!(
                        got.is_none() || got == unshared.get(i),
                        "slot {} reads {:?}, unshared {:?}",
                        i,
                        got,
                        unshared.get(i)
                    );
                }
            }
        }
    }
}
