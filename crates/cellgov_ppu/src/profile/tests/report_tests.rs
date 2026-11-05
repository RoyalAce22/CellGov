//! Static counts, coverage of a candidate set, and the report's JSON.

use super::{coverage, static_counts, InsnClass, OperandProfile, ProfileReport};
use crate::instruction::PpuInstruction;

/// `addi rT, rT, 1`.
fn addi(rt: u32) -> u32 {
    (14 << 26) | (rt << 21) | (rt << 16) | 1
}

/// `addi rT, rT, 0`: a class of its own, its immediate zero.
fn addi_zero(rt: u32) -> u32 {
    (14 << 26) | (rt << 21) | (rt << 16)
}

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

/// The `rt` of an `addi`.
fn rt(insn: &PpuInstruction) -> Option<u8> {
    match *insn {
        PpuInstruction::Addi { rt, .. } => Some(rt),
        _ => None,
    }
}

#[test]
fn static_counts_count_each_decoded_word_once() {
    let text = bytes(&[addi(3), addi(4), addi_zero(4), 0]);
    let counts = static_counts(&text);
    assert_eq!(counts.get(&InsnClass::of(addi(3)).unwrap()), Some(&2));
    assert_eq!(counts.get(&InsnClass::of(addi_zero(4)).unwrap()), Some(&1));
    assert_eq!(
        counts.values().sum::<u64>(),
        3,
        "the zero word has no decode"
    );
}

#[test]
fn coverage_takes_the_first_pair_of_an_overlapping_run() {
    // addi r3; addi r4; addi r5; addi r6 at 0..16. The candidate pairs
    // are (r3, r4) and (r4, r5), which overlap, so the walk fuses the
    // first. The candidate quickening is r6 alone.
    let text = bytes(&[addi(3), addi(4), addi(5), addi(6)]);
    let mut profile = OperandProfile::default();
    for (pc, n) in [(0u64, 5u32), (4, 5), (8, 5), (12, 2)] {
        for _ in 0..n {
            profile.record(pc);
        }
    }
    let fuses = |a: &PpuInstruction, b: &PpuInstruction| {
        matches!((rt(a), rt(b)), (Some(3), Some(4)) | (Some(4), Some(5)))
    };
    let covers = |a: &PpuInstruction| rt(a) == Some(6);

    let c = coverage(&profile, 0, &text, covers, fuses);

    assert_eq!(c.retired, 17);
    assert_eq!(
        c.dispatches_removed, 5,
        "one per retirement of the head at 0"
    );
    assert_eq!(
        c.covered,
        10 + 2,
        "the pair at 0 and the quickened word at 12"
    );
}

#[test]
fn coverage_leaves_a_pair_second_slot_to_its_stored_word() {
    // The pair at 0 claims 4, so a candidate quickening of the word at
    // 4 covers nothing there.
    let text = bytes(&[addi(3), addi(4)]);
    let mut profile = OperandProfile::default();
    profile.record(0);
    profile.record(4);
    profile.record(4);
    let fuses = |a: &PpuInstruction, b: &PpuInstruction| rt(a) == Some(3) && rt(b) == Some(4);
    let covers = |a: &PpuInstruction| rt(a) == Some(4);

    let c = coverage(&profile, 0, &text, covers, fuses);

    assert_eq!(c.covered, 2);
    assert_eq!(c.dispatches_removed, 1);
}

#[test]
fn shipped_coverage_counts_fused_heads_and_quickened_slots_and_skips_consumed_slots() {
    // lwz r3, 8(r1); cmpwi cr0, r3, 42 fuse at 0. li r5, 7 quickens at 8.
    // addi r5, r5, 1 at 12 stays as decoded.
    let lwz = (32 << 26) | (3 << 21) | (1 << 16) | 8;
    let cmpwi = (11 << 26) | (3 << 16) | 42;
    let li = (14 << 26) | (5 << 21) | 7;
    let text = bytes(&[lwz, cmpwi, li, addi(5)]);
    let shadow = crate::shadow::PredecodedShadow::build(0, &text);
    let mut profile = OperandProfile::default();
    // The fused run counts both halves; a branch then reaches 4 alone.
    for pc in [0, 4, 8, 12, 4] {
        profile.record(pc);
    }

    let c = super::shipped_coverage(&profile, &shadow, &text);

    assert_eq!(c.retired, 5);
    assert_eq!(c.dispatches_removed, 1, "one per retirement of the head");
    assert_eq!(
        c.covered,
        2 + 1,
        "the pair at 0 and the li at 8; the consumed slot at 4 runs its stored word"
    );
}

#[test]
fn the_report_ranks_by_retirements_and_carries_the_static_count() {
    let text = bytes(&[addi(3), addi_zero(4), addi_zero(4)]);
    let mut profile = OperandProfile::default();
    for pc in [0, 4, 8, 0, 0] {
        profile.record(pc);
    }

    let report = ProfileReport::new(&profile, &text, &text, None);

    let nonzero = InsnClass::of(addi(3)).unwrap();
    let zero = InsnClass::of(addi_zero(4)).unwrap();
    assert_eq!(report.retired, 5);
    assert_eq!(report.insns, vec![(nonzero, 3, 1), (zero, 2, 2)]);
    // 0 -> 4 -> 8 ran in order once, then 8 -> 0 is a jump.
    assert_eq!(report.pairs.iter().map(|(_, n)| n).sum::<u64>(), 2);
    assert_eq!(
        report.to_json(),
        "{\n  \"retired\": 5,\n  \"insns\": [\
         \n    {\"class\": \"Addi imm=nz same\", \"dynamic\": 3, \"static\": 1},\
         \n    {\"class\": \"Addi imm=0 same\", \"dynamic\": 2, \"static\": 2}\
         \n  ],\n  \"pairs\": [\
         \n    {\"pair\": \"Addi imm=nz same ; Addi imm=0 same\", \"dynamic\": 1},\
         \n    {\"pair\": \"Addi imm=0 same ; Addi imm=0 same | dependent\", \"dynamic\": 1}\
         \n  ],\n  \"shipped\": null,\n  \"shipped_forms\": [\n  ]\n}\n"
    );
}
