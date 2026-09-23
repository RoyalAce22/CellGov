//! Generated-input properties of the shadow's invalidation: a stale
//! range is a partition, and a fused pair is never half stale.

use proptest::prelude::*;

use super::PredecodedShadow;
use crate::instruction::encode::encode;
use crate::instruction::PpuInstruction;
use crate::shadow::semantics_support::reg;

/// Guest address of the shadowed range.
const BASE: u64 = 0x1000;

/// Words that quicken and fuse, with plain fillers between them.
fn word() -> impl Strategy<Value = u32> {
    let insn = prop_oneof![
        (reg(), any::<i16>()).prop_map(|(rt, imm)| PpuInstruction::Addi { rt, ra: 0, imm }),
        (reg(), reg(), -64i16..64).prop_map(|(rt, ra, imm)| PpuInstruction::Lwz { rt, ra, imm }),
        (0u8..8, reg(), -64i16..64).prop_map(|(bf, ra, imm)| PpuInstruction::Cmpwi { bf, ra, imm }),
        (reg(), reg(), -64i16..64).prop_map(|(rs, ra, imm)| PpuInstruction::Stw { rs, ra, imm }),
        (reg(), reg(), -16i16..16).prop_map(|(rs, ra, d)| PpuInstruction::Std {
            rs,
            ra,
            imm: d * 4
        }),
        reg().prop_map(|rt| PpuInstruction::Mflr { rt }),
        reg().prop_map(|rs| PpuInstruction::Mtlr { rs }),
        (reg(), reg(), -16i16..16).prop_map(|(bo, bi, d)| PpuInstruction::Bc {
            bo,
            bi,
            offset: d * 4,
            aa: false,
            link: false,
        }),
        (reg(), reg(), reg()).prop_map(|(ra, rs, rb)| PpuInstruction::Or {
            ra,
            rs,
            rb,
            rc: false
        }),
    ];
    insn.prop_filter_map("encodable", |insn| encode(&insn).ok())
}

#[derive(Debug, Clone, Copy)]
enum Op {
    Invalidate { offset: i64, len: u64 },
    Refresh { slot: usize, raw: u32 },
}

fn op(slots: usize) -> impl Strategy<Value = Op> {
    let span = slots as i64 * 4;
    prop_oneof![
        3 => (-16i64..span + 16, 0u64..=24).prop_map(|(offset, len)| Op::Invalidate { offset, len }),
        1 => (0..slots, word()).prop_map(|(slot, raw)| Op::Refresh { slot, raw }),
    ]
}

/// Two words the pairing pass fuses.
fn pair_words() -> impl Strategy<Value = [u32; 2]> {
    prop_oneof![
        (reg(), reg(), -64i16..64, 0u8..8, any::<i16>()).prop_filter_map(
            "encodable",
            |(rt, ra, imm, bf, cmp)| Some([
                encode(&PpuInstruction::Lwz { rt, ra, imm }).ok()?,
                encode(&PpuInstruction::Cmpwi {
                    bf,
                    ra: rt,
                    imm: cmp
                })
                .ok()?,
            ])
        ),
        (reg(), any::<i16>(), reg(), -64i16..64).prop_filter_map(
            "encodable",
            |(rt, imm, ra, off)| Some([
                encode(&PpuInstruction::Addi { rt, ra: 0, imm }).ok()?,
                encode(&PpuInstruction::Stw {
                    rs: rt,
                    ra,
                    imm: off
                })
                .ok()?,
            ])
        ),
    ]
}

/// Random words with one fusable pair spliced in, so every program
/// carries at least one pair for the partition to protect.
fn program() -> impl Strategy<Value = Vec<u32>> {
    (
        proptest::collection::vec(word(), 0..44),
        pair_words(),
        any::<prop::sample::Index>(),
    )
        .prop_map(|(mut words, pair, at)| {
            let at = at.index(words.len() + 1);
            words.splice(at..at, pair);
            words
        })
}

fn build(words: &[u32]) -> PredecodedShadow {
    let mut bytes = Vec::with_capacity(words.len() * 4);
    for w in words {
        bytes.extend_from_slice(&w.to_be_bytes());
    }
    PredecodedShadow::build(BASE, &bytes)
}

fn stale_set(shadow: &PredecodedShadow) -> Vec<bool> {
    (0..shadow.len())
        .map(|i| shadow.slot(i).expect("index in range").1)
        .collect()
}

/// The pair invariant, in both directions:
///
/// - a fresh fused head has a fresh `Consumed` in the next slot;
/// - a fresh `Consumed` has a fresh fused head in the slot before it.
fn pairs_are_whole(shadow: &PredecodedShadow) -> Result<(), TestCaseError> {
    for i in 0..shadow.len() {
        let (insn, stale) = shadow.slot(i).expect("index in range");
        if stale {
            continue;
        }
        match insn {
            Some(head) if head.is_super_pair() => {
                let partner = shadow.slot(i + 1);
                prop_assert_eq!(
                    partner,
                    Some((Some(PpuInstruction::Consumed), false)),
                    "fresh head at slot {} has partner {:?}",
                    i,
                    partner
                );
            }
            Some(PpuInstruction::Consumed) => {
                let head = i.checked_sub(1).and_then(|h| shadow.slot(h));
                let whole = matches!(head, Some((Some(insn), false)) if insn.is_super_pair());
                prop_assert!(whole, "fresh Consumed at slot {} behind {:?}", i, head);
            }
            _ => {}
        }
    }
    Ok(())
}

/// `get` answers exactly the fresh slots.
fn get_matches_slots(shadow: &PredecodedShadow) -> Result<(), TestCaseError> {
    for i in 0..shadow.len() {
        let (insn, stale) = shadow.slot(i).expect("index in range");
        let expected = if stale { None } else { insn };
        prop_assert_eq!(shadow.get(BASE + 4 * i as u64), expected, "slot {}", i);
    }
    Ok(())
}

proptest! {
    #[test]
    fn invalidation_stales_the_range_and_never_splits_a_pair(
        words in program(),
        ops in proptest::collection::vec(op(48), 1..24),
    ) {
        let mut shadow = build(&words);
        let slots = shadow.len();
        prop_assert!(
            (0..slots).any(|i| shadow.slot(i).is_some_and(|(insn, _)| insn.is_some_and(|x| x.is_super_pair()))),
            "the program fused no pair"
        );
        pairs_are_whole(&shadow)?;
        for op in ops {
            let before = stale_set(&shadow);
            match op {
                Op::Invalidate { offset, len } => {
                    let addr = (BASE as i64 + offset) as u64;
                    shadow.invalidate_range(addr, len);
                    let after = stale_set(&shadow);
                    let end = addr.saturating_add(len);
                    let overlaps = |i: usize| {
                        let slot_start = BASE + 4 * i as u64;
                        len > 0 && slot_start < end && slot_start + 4 > addr
                    };
                    let consumed = |i: usize| {
                        shadow.slot(i).is_some_and(|(insn, _)| insn == Some(PpuInstruction::Consumed))
                    };
                    for i in 0..slots {
                        if overlaps(i) {
                            prop_assert!(after[i], "slot {} under [{:#x}, {:#x}) is fresh", i, addr, end);
                        }
                        prop_assert!(!before[i] || after[i], "slot {} was un-staled by an invalidate", i);
                        // A slot outside the range goes stale only as
                        // the other half of a pair the range touched.
                        if after[i] && !before[i] && !overlaps(i) {
                            let head_of_written_partner = consumed(i + 1) && overlaps(i + 1);
                            let partner_of_written_head =
                                consumed(i) && i.checked_sub(1).is_some_and(overlaps);
                            prop_assert!(
                                head_of_written_partner || partner_of_written_head,
                                "slot {} outside [{:#x}, {:#x}) was staled",
                                i,
                                addr,
                                end
                            );
                        }
                    }
                }
                Op::Refresh { slot, raw } => {
                    let slot = slot % slots;
                    let pc = BASE + 4 * slot as u64;
                    // The fetch loop refreshes only a slot `get` could
                    // not answer; a refresh of a fresh fused head or
                    // its partner would split the pair.
                    if shadow.get(pc).is_some() {
                        continue;
                    }
                    let refreshed = shadow.refresh(pc, raw);
                    prop_assert!(refreshed.is_some());
                    let after = stale_set(&shadow);
                    for i in 0..slots {
                        if i == slot {
                            prop_assert!(!after[i]);
                        } else {
                            prop_assert_eq!(after[i], before[i], "slot {} changed under a refresh of slot {}", i, slot);
                        }
                    }
                }
            }
            pairs_are_whole(&shadow)?;
            get_matches_slots(&shadow)?;
        }
    }
}
