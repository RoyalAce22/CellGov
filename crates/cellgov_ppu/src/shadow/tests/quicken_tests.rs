//! Quickening rewrites generic encodings into specialized instruction variants.

use crate::instruction::PpuInstruction;
use crate::shadow::test_support::{
    build_from_words, cmpwi_raw, li_raw, or_raw, ori_raw, rldicl_raw, rldicr_raw, rlwinm_raw,
};

#[test]
fn quicken_addi_ra0_becomes_li() {
    // addi r3, r0, 42 => Li { rt: 3, imm: 42 }
    let shadow = build_from_words(0, &[li_raw(3, 42)]);
    assert_eq!(shadow.get(0), Some(PpuInstruction::Li { rt: 3, imm: 42 }));
}

#[test]
fn quicken_or_same_reg_becomes_mr() {
    // or r3, r4, r4 => Mr { ra: 3, rs: 4 }
    let shadow = build_from_words(0, &[or_raw(4, 3, 4)]);
    assert_eq!(shadow.get(0), Some(PpuInstruction::Mr { ra: 3, rs: 4 }));
}

#[test]
fn quicken_rlwinm_slwi() {
    // rlwinm r3, r4, 8, 0, 23 => Slwi { ra: 3, rs: 4, n: 8 }
    let shadow = build_from_words(0, &[rlwinm_raw(4, 3, 8, 0, 23)]);
    assert_eq!(
        shadow.get(0),
        Some(PpuInstruction::Slwi { ra: 3, rs: 4, n: 8 })
    );
}

#[test]
fn quicken_rlwinm_srwi() {
    // srwi r3, r4, 8 => rlwinm r3, r4, 24, 8, 31
    let shadow = build_from_words(0, &[rlwinm_raw(4, 3, 24, 8, 31)]);
    assert_eq!(
        shadow.get(0),
        Some(PpuInstruction::Srwi { ra: 3, rs: 4, n: 8 })
    );
}

#[test]
fn quicken_rlwinm_clrlwi() {
    // clrlwi r3, r4, 16 => rlwinm r3, r4, 0, 16, 31
    let shadow = build_from_words(0, &[rlwinm_raw(4, 3, 0, 16, 31)]);
    assert_eq!(
        shadow.get(0),
        Some(PpuInstruction::Clrlwi {
            ra: 3,
            rs: 4,
            n: 16
        })
    );
}

#[test]
fn quicken_non_specializable_unchanged() {
    // or r3, r4, r5 (rs != rb, not mr) stays as Or
    let shadow = build_from_words(0, &[or_raw(4, 3, 5)]);
    assert_eq!(
        shadow.get(0),
        Some(PpuInstruction::Or {
            ra: 3,
            rs: 4,
            rb: 5,
            rc: false,
        })
    );
}

#[test]
fn quicken_addi_nonzero_ra_unchanged() {
    // addi r3, r5, 42 (ra != 0) stays as Addi
    let raw = (14 << 26) | (3 << 21) | (5 << 16) | (42u16 as u32);
    let shadow = build_from_words(0, &[raw]);
    assert_eq!(
        shadow.get(0),
        Some(PpuInstruction::Addi {
            rt: 3,
            ra: 5,
            imm: 42
        })
    );
}

#[test]
fn refresh_applies_quickening() {
    let mut shadow = build_from_words(0, &[li_raw(3, 10)]);
    assert_eq!(shadow.get(0), Some(PpuInstruction::Li { rt: 3, imm: 10 }));
    // Invalidate and refresh with a new li instruction
    shadow.invalidate_range(0, 4);
    assert!(shadow.get(0).is_none());
    let refreshed = shadow.refresh(0, li_raw(4, 99));
    assert_eq!(refreshed, Some(Some(PpuInstruction::Li { rt: 4, imm: 99 })));
    assert_eq!(shadow.get(0), Some(PpuInstruction::Li { rt: 4, imm: 99 }));
}

#[test]
fn refresh_applies_quickening_or_to_mr() {
    let mut shadow = build_from_words(0, &[li_raw(3, 1)]);
    shadow.invalidate_range(0, 4);
    // Refresh with `or r5, r6, r6` => should quicken to Mr
    let refreshed = shadow.refresh(0, or_raw(6, 5, 6));
    assert_eq!(refreshed, Some(Some(PpuInstruction::Mr { ra: 5, rs: 6 })));
}

#[test]
fn quicken_ori_same_reg_zero_becomes_nop() {
    // ori r5, r5, 0 => Nop
    let shadow = build_from_words(0, &[ori_raw(5, 5, 0)]);
    assert_eq!(shadow.get(0), Some(PpuInstruction::Nop));
}

#[test]
fn quicken_ori_different_reg_unchanged() {
    // ori r3, r5, 0 -- different regs, not nop
    let shadow = build_from_words(0, &[ori_raw(5, 3, 0)]);
    assert!(matches!(shadow.get(0), Some(PpuInstruction::Ori { .. })));
}

#[test]
fn quicken_ori_nonzero_imm_unchanged() {
    // ori r5, r5, 1 -- nonzero imm, not nop
    let shadow = build_from_words(0, &[ori_raw(5, 5, 1)]);
    assert!(matches!(shadow.get(0), Some(PpuInstruction::Ori { .. })));
}

#[test]
fn quicken_cmpwi_zero_becomes_cmpw_zero() {
    // cmpwi cr0, r3, 0 => CmpwZero { bf: 0, ra: 3 }
    let shadow = build_from_words(0, &[cmpwi_raw(0, 3, 0)]);
    assert_eq!(
        shadow.get(0),
        Some(PpuInstruction::CmpwZero { bf: 0, ra: 3 })
    );
}

#[test]
fn quicken_cmpwi_nonzero_unchanged() {
    // cmpwi cr0, r3, 42 -- nonzero imm, stays Cmpwi
    let shadow = build_from_words(0, &[cmpwi_raw(0, 3, 42)]);
    assert!(matches!(shadow.get(0), Some(PpuInstruction::Cmpwi { .. })));
}

#[test]
fn quicken_rldicl_sh0_becomes_clrldi() {
    // rldicl r3, r4, 0, 32 => Clrldi { ra: 3, rs: 4, n: 32 }
    let shadow = build_from_words(0, &[rldicl_raw(4, 3, 0, 32)]);
    assert_eq!(
        shadow.get(0),
        Some(PpuInstruction::Clrldi {
            ra: 3,
            rs: 4,
            n: 32
        })
    );
}

#[test]
fn quicken_rldicr_sldi_pattern() {
    // sldi r3, r4, 8 => rldicr r3, r4, 8, 55
    let shadow = build_from_words(0, &[rldicr_raw(4, 3, 8, 55)]);
    assert_eq!(
        shadow.get(0),
        Some(PpuInstruction::Sldi { ra: 3, rs: 4, n: 8 })
    );
}

#[test]
fn quicken_rldicl_srdi_pattern() {
    // srdi r3, r4, 8 => rldicl r3, r4, 56, 8
    let shadow = build_from_words(0, &[rldicl_raw(4, 3, 56, 8)]);
    assert_eq!(
        shadow.get(0),
        Some(PpuInstruction::Srdi { ra: 3, rs: 4, n: 8 })
    );
}

#[test]
fn quicken_rldicl_nonzero_sh_non_srdi_unchanged() {
    // rldicl with sh != 0 and mb != 64-sh stays as Rldicl
    let shadow = build_from_words(0, &[rldicl_raw(4, 3, 10, 20)]);
    assert!(matches!(shadow.get(0), Some(PpuInstruction::Rldicl { .. })));
}

#[test]
fn quicken_or_same_reg_to_self_is_not_mr() {
    // `or rN, rN, rN` is the CBE PPE thread-priority / dispatch-stall
    // hint family (cctpl/cctpm/cctph for r1/r2/r3 etc.); quickening to
    // Mr { ra: N, rs: N } would erase the architectural side effect.
    let shadow = build_from_words(0, &[or_raw(1, 1, 1)]);
    assert!(
        matches!(
            shadow.get(0),
            Some(PpuInstruction::Or {
                ra: 1,
                rs: 1,
                rb: 1,
                rc: false,
            })
        ),
        "or 1, 1, 1 (cctpl priority hint) must not be quickened to Mr"
    );
}

#[test]
fn quicken_or_dot_form_with_same_reg_is_not_mr() {
    // or. rA, rS, rS keeps CR0 update; the Rc=0 guard on the Mr arm
    // must stop it from collapsing to Mr (which discards the CR0 write).
    // Manual dot-form encoding: same as or_raw but with bit 0 set.
    let raw = or_raw(4, 3, 4) | 1;
    let shadow = build_from_words(0, &[raw]);
    assert!(
        matches!(shadow.get(0), Some(PpuInstruction::Or { rc: true, .. })),
        "or. rA, rS, rS must remain Or with rc=true, not quicken to Mr"
    );
}

#[test]
fn quicken_rlwinm_sh_zero_routes_to_clrlwi_not_slwi() {
    // rlwinm rA, rS, 0, 0, 31 is a 32-bit zero-extend. Without a
    // sh==0 guard on the Slwi quickening arm it can match this
    // form and produce Slwi { n: 0 }; the guarded routing emits
    // Clrlwi { n: 0 }, which is the canonical idiom.
    let shadow = build_from_words(0, &[rlwinm_raw(4, 3, 0, 0, 31)]);
    assert_eq!(
        shadow.get(0),
        Some(PpuInstruction::Clrlwi { ra: 3, rs: 4, n: 0 })
    );
}

#[test]
fn quicken_rlwinm_dot_form_not_quickened() {
    // rlwinm. preserves CR0 update; every quickening guard requires
    // Rc=0, so the dot form must remain a raw Rlwinm dispatch.
    let raw = rlwinm_raw(4, 3, 8, 0, 23) | 1;
    let shadow = build_from_words(0, &[raw]);
    assert!(
        matches!(shadow.get(0), Some(PpuInstruction::Rlwinm { rc: true, .. })),
        "rlwinm. must not be quickened; CR0 update would be lost"
    );
}
