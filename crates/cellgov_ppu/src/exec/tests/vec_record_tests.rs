//! Recording (Rc=1) VXR vector compares and their CR6 result.

use super::*;
use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;

// Field 6 starts at 0b0101, which no recording compare can produce.
const CR_SEED: u32 = 0xABCD_EF51;

fn words(lanes: [u32; 4]) -> u128 {
    let mut r = [0u8; 16];
    for (i, v) in lanes.iter().enumerate() {
        r[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    u128::from_be_bytes(r)
}

/// Runs `vcmpequw. v3,v1,v2` over a CR seeded with `CR_SEED`.
fn vcmpequw_dot(a: [u32; 4], b: [u32; 4]) -> PpuState {
    let mut s = PpuState::new();
    s.set_cr(CR_SEED);
    s.set_vr(1, words(a));
    s.set_vr(2, words(b));
    let v = exec_no_mem(
        &PpuInstruction::Vx {
            op: VxOp::Vcmpequw,
            rc: true,
            vt: 3,
            va: 1,
            vb: 2,
        },
        &mut s,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    s
}

/// CR with field 6 replaced by `cr6` and every other field from `CR_SEED`.
fn seed_with_cr6(cr6: u32) -> u32 {
    (CR_SEED & !0x0000_00F0) | (cr6 << 4)
}

// [AltiVec-PEM p:6-56 s:6.2] vcmpequw.: CR6 = all_equal || 0b0 || none_equal || 0b0.
#[test]
fn vcmpequw_dot_all_equal_sets_cr6_bit_0() {
    let s = vcmpequw_dot([1, 2, 3, 4], [1, 2, 3, 4]);
    assert_eq!(s.vr[3], words([0xFFFF_FFFF; 4]));
    assert_eq!(s.cr(), seed_with_cr6(0b1000));
}

#[test]
fn vcmpequw_dot_none_equal_sets_cr6_bit_2() {
    let s = vcmpequw_dot([1, 2, 3, 4], [5, 6, 7, 8]);
    assert_eq!(s.vr[3], 0);
    assert_eq!(s.cr(), seed_with_cr6(0b0010));
}

#[test]
fn vcmpequw_dot_partial_match_clears_cr6() {
    let s = vcmpequw_dot([1, 2, 3, 4], [1, 0, 3, 0]);
    assert_eq!(s.vr[3], words([0xFFFF_FFFF, 0, 0xFFFF_FFFF, 0]));
    assert_eq!(s.cr(), seed_with_cr6(0));
}

#[test]
fn an_unimplemented_recording_compare_faults_without_writing_cr() {
    let mut s = PpuState::new();
    s.set_cr(CR_SEED);
    let v = exec_no_mem(
        &PpuInstruction::Vx {
            op: VxOp::Vcmpequb,
            rc: true,
            vt: 3,
            va: 1,
            vb: 2,
        },
        &mut s,
    );
    assert_eq!(
        v,
        ExecuteVerdict::Fault(PpuFault::UnimplementedInstruction(0x406))
    );
    assert_eq!(s.cr(), CR_SEED);
}

// [AltiVec-PEM p:6-51 s:6.2] vcmpbfp.: CR6 = 0b00 || all_within_bounds || 0.
#[test]
fn vcmpbfp_record_sets_only_the_within_bounds_bit() {
    assert_eq!(record_cr6(VxOp::Vcmpbfp, 0), 0b0010);
    assert_eq!(record_cr6(VxOp::Vcmpbfp, words([0, 0x8000_0000, 0, 0])), 0);
    assert_eq!(record_cr6(VxOp::Vcmpbfp, u128::MAX), 0);
}
