//! A refresh of one half of a fused pair stales the other half.

use super::*;
use crate::instruction::FusedPair;
use crate::shadow::test_support::{build_from_words, cmpwi_raw, li_raw, lwz_raw};

/// `lwz` + `cmpwi` fused at slot 0, `Consumed` at slot 1, `li` at slot 2.
fn fused() -> PredecodedShadow {
    let shadow = build_from_words(0, &[lwz_raw(3, 1, 8), cmpwi_raw(0, 3, 42), li_raw(5, 7)]);
    assert!(matches!(
        shadow.get(0),
        Some(PpuInstruction::Fused(FusedPair::LwzCmpwi { .. }))
    ));
    assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
    shadow
}

#[test]
fn refreshing_a_fresh_head_stales_its_consumed_partner() {
    let mut shadow = fused();
    shadow.refresh(0, li_raw(3, 1));
    assert_eq!(shadow.get(0), Some(PpuInstruction::Li { rt: 3, imm: 1 }));
    assert_eq!(shadow.get(4), None, "a fresh Consumed would skip slot 1");
    assert!(shadow.get(8).is_some());
}

#[test]
fn refreshing_a_fresh_consumed_stales_its_head() {
    let mut shadow = fused();
    shadow.refresh(4, li_raw(4, 2));
    assert_eq!(shadow.get(0), None, "a fresh head would run slot 1 twice");
    assert_eq!(shadow.get(4), Some(PpuInstruction::Li { rt: 4, imm: 2 }));
    assert!(shadow.get(8).is_some());
}

#[test]
fn refreshing_a_stale_pair_head_then_partner_leaves_both_fresh() {
    let mut shadow = fused();
    shadow.invalidate_range(0, 8);
    shadow.refresh(0, li_raw(3, 1));
    shadow.refresh(4, li_raw(4, 2));
    assert_eq!(shadow.get(0), Some(PpuInstruction::Li { rt: 3, imm: 1 }));
    assert_eq!(shadow.get(4), Some(PpuInstruction::Li { rt: 4, imm: 2 }));
}
