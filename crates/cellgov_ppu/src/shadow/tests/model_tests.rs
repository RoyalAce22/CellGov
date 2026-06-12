//! Shadow code-cache slot lookup and range invalidation.

use super::*;
use crate::shadow::test_support::{
    b_raw, build_from_words, cmpwi_raw, li_raw, lwz_raw, sc_raw, stw_raw,
};

#[test]
fn build_decodes_all_slots() {
    let shadow = build_from_words(0, &[li_raw(3, 10), li_raw(4, 20)]);
    assert_eq!(shadow.len(), 2);
    assert_eq!(shadow.base(), 0);
    assert_eq!(shadow.end(), 8);
    assert!(shadow.get(0).is_some());
    assert!(shadow.get(4).is_some());
}

#[test]
fn get_returns_none_for_out_of_range() {
    let shadow = build_from_words(0x100, &[li_raw(3, 42)]);
    assert!(shadow.get(0x0FC).is_none());
    assert!(shadow.get(0x100).is_some());
    assert!(shadow.get(0x104).is_none());
}

#[test]
fn get_returns_none_for_misaligned_pc() {
    let shadow = build_from_words(0, &[li_raw(3, 1), li_raw(4, 2)]);
    assert!(shadow.get(2).is_none());
}

#[test]
fn invalidate_marks_slots_stale() {
    let mut shadow = build_from_words(0, &[li_raw(3, 10), li_raw(4, 20), li_raw(5, 30)]);
    assert!(shadow.get(0).is_some());
    assert!(shadow.get(4).is_some());
    assert!(shadow.get(8).is_some());

    // Byte range 2..6 overlaps slots 0 and 1.
    shadow.invalidate_range(2, 4);
    assert!(shadow.get(0).is_none());
    assert!(shadow.get(4).is_none());
    assert!(shadow.get(8).is_some());
}

#[test]
fn invalidate_outside_range_is_noop() {
    let mut shadow = build_from_words(0x100, &[li_raw(3, 1)]);
    shadow.invalidate_range(0, 0x100);
    assert!(shadow.get(0x100).is_some());
    shadow.invalidate_range(0x200, 0x100);
    assert!(shadow.get(0x100).is_some());
}

#[test]
fn refresh_clears_stale_and_updates_slot() {
    let mut shadow = build_from_words(0, &[li_raw(3, 10)]);
    shadow.invalidate_range(0, 4);
    assert!(shadow.get(0).is_none());

    let new_raw = li_raw(3, 99);
    let insn = shadow.refresh(0, new_raw);
    // Quickening applies: addi r3, r0, 99 => Li.
    assert_eq!(insn, Some(Some(PpuInstruction::Li { rt: 3, imm: 99 })));
    assert!(shadow.get(0).is_some());
}

#[test]
fn refresh_out_of_range_returns_none() {
    let mut shadow = build_from_words(0x100, &[li_raw(3, 1)]);
    assert!(shadow.refresh(0, li_raw(3, 1)).is_none());
    assert!(shadow.refresh(0x104, li_raw(3, 1)).is_none());
}

#[test]
fn empty_shadow_is_empty() {
    let shadow = PredecodedShadow::build(0, &[]);
    assert!(shadow.is_empty());
    assert_eq!(shadow.len(), 0);
    assert!(shadow.get(0).is_none());
}

#[test]
fn invalidate_zero_length_is_noop() {
    let mut shadow = build_from_words(0, &[li_raw(3, 1)]);
    shadow.invalidate_range(0, 0);
    assert!(shadow.get(0).is_some());
}

#[test]
fn invalidate_partial_byte_within_slot_stales_that_slot() {
    let mut shadow = build_from_words(0, &[li_raw(3, 1)]);
    shadow.invalidate_range(3, 1);
    assert!(
        shadow.get(0).is_none(),
        "1-byte write inside slot must stale it"
    );
}

#[test]
fn block_len_straight_line() {
    let shadow = build_from_words(0, &[li_raw(3, 1), li_raw(4, 2), li_raw(5, 3), li_raw(6, 4)]);
    assert_eq!(shadow.block_len_at(0), 4);
    assert_eq!(shadow.block_len_at(4), 3);
    assert_eq!(shadow.block_len_at(8), 2);
    assert_eq!(shadow.block_len_at(12), 1);
}

#[test]
fn block_len_branch_terminates() {
    let shadow = build_from_words(0, &[li_raw(3, 1), li_raw(4, 2), b_raw(8), li_raw(5, 3)]);
    assert_eq!(shadow.block_len_at(0), 3);
    assert_eq!(shadow.block_len_at(4), 2);
    assert_eq!(shadow.block_len_at(8), 1);
    assert_eq!(shadow.block_len_at(12), 1);
}

#[test]
fn block_len_syscall_terminates() {
    let shadow = build_from_words(0, &[li_raw(3, 1), sc_raw(), li_raw(4, 2)]);
    assert_eq!(shadow.block_len_at(0), 2);
    assert_eq!(shadow.block_len_at(4), 1);
    assert_eq!(shadow.block_len_at(8), 1);
}

#[test]
fn block_len_invalidation_resets() {
    let mut shadow = build_from_words(0, &[li_raw(3, 1), li_raw(4, 2), li_raw(5, 3)]);
    assert_eq!(shadow.block_len_at(0), 3);
    shadow.invalidate_range(4, 4);
    // Predecessor block_len is an upper bound post-invalidation.
    assert_eq!(shadow.block_len_at(0), 3);
    assert_eq!(shadow.block_len_at(4), 1);
    assert_eq!(shadow.block_len_at(8), 1);
}

#[test]
fn block_len_refresh_rescans() {
    let mut shadow = build_from_words(0, &[li_raw(3, 1), li_raw(4, 2), li_raw(5, 3)]);
    assert_eq!(shadow.block_len_at(0), 3);
    shadow.invalidate_range(4, 4);
    assert_eq!(shadow.block_len_at(4), 1);
    shadow.refresh(4, li_raw(4, 99));
    assert_eq!(shadow.block_len_at(4), 2);
    assert_eq!(shadow.block_len_at(0), 3);
}

#[test]
fn block_len_out_of_range_returns_one() {
    let shadow = build_from_words(0x100, &[li_raw(3, 1)]);
    assert_eq!(shadow.block_len_at(0), 1);
    assert_eq!(shadow.block_len_at(0x200), 1);
}

#[test]
fn block_len_empty_shadow() {
    let shadow = PredecodedShadow::build(0, &[]);
    assert_eq!(shadow.block_len_at(0), 1);
}

#[test]
fn invalidate_just_consumed_widens_to_super_pair_head() {
    // lwz + cmpwi fused at slot 0, Consumed at slot 4. Invalidating
    // only the Consumed slot must widen to include the head;
    // otherwise the head's fused dispatch would still execute (firing
    // both the lwz and the cmpwi) and the freshly-written instruction
    // at slot 4 would also execute -- a double-execute.
    let mut shadow = build_from_words(0, &[lwz_raw(3, 1, 8), cmpwi_raw(0, 3, 42)]);
    assert!(matches!(
        shadow.get(0),
        Some(PpuInstruction::LwzCmpwi { .. })
    ));
    assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
    // Invalidate ONLY slot 4 (byte range [4..8)).
    shadow.invalidate_range(4, 4);
    assert!(
        shadow.get(0).is_none(),
        "super-pair head must be staled when its Consumed partner is invalidated"
    );
    assert!(shadow.get(4).is_none());
}

#[test]
fn invalidate_just_super_pair_head_widens_to_consumed() {
    // Symmetric case: invalidate only the head (slot 0) and verify
    // the Consumed at slot 4 also goes stale. Otherwise the fetch
    // loop would skip slot 4 forever (treating Consumed as the
    // already-retired second half of a pair the head no longer
    // represents).
    let mut shadow = build_from_words(0, &[lwz_raw(3, 1, 8), cmpwi_raw(0, 3, 42)]);
    assert!(matches!(
        shadow.get(0),
        Some(PpuInstruction::LwzCmpwi { .. })
    ));
    assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
    // Invalidate ONLY slot 0 (byte range [0..4)).
    shadow.invalidate_range(0, 4);
    assert!(shadow.get(0).is_none());
    assert!(
        shadow.get(4).is_none(),
        "Consumed partner must be staled when its super-pair head is invalidated"
    );
}

#[test]
fn invalidate_partner_widening_preserves_unrelated_pairs() {
    // Two adjacent pairs at slots 0..2 and 2..4. Invalidating only
    // slot 1 (the Consumed of pair A) must widen to slot 0 (head of
    // pair A) but not touch pair B at slots 2..4.
    let mut shadow = build_from_words(
        0,
        &[
            li_raw(3, 1),
            stw_raw(3, 1, 0),
            li_raw(4, 2),
            stw_raw(4, 1, 8),
        ],
    );
    assert!(matches!(shadow.get(0), Some(PpuInstruction::LiStw { .. })));
    assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
    assert!(matches!(shadow.get(8), Some(PpuInstruction::LiStw { .. })));
    assert_eq!(shadow.get(12), Some(PpuInstruction::Consumed));
    // Invalidate only slot 1 (the Consumed at byte 4).
    shadow.invalidate_range(4, 4);
    assert!(shadow.get(0).is_none(), "pair A head must be staled");
    assert!(shadow.get(4).is_none(), "pair A Consumed must be staled");
    assert!(shadow.get(8).is_some(), "pair B head must remain intact");
    assert_eq!(
        shadow.get(12),
        Some(PpuInstruction::Consumed),
        "pair B Consumed must remain intact"
    );
}
