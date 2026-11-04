//! `vmaxfp` and `vminfp` in both NJ modes, against a reference written
//! from the PEM text and the exact order of the compare tests.

use super::float_compare_tests::{classes, order};
use super::lane_support::{lane, pack_u32x4, with_lane};
use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::{VSCR_NJ, VSCR_SAT};
use std::cmp::Ordering;

/// vD after `op` on vA, vB = `a`, `b` with VSCR[NJ] = `nj`; checks that
/// SAT and CR do not change.
fn run(op: VxOp, nj: bool, a: u128, b: u128) -> u128 {
    let mut s = PpuState::new();
    s.set_vscr(if nj { VSCR_NJ } else { 0 });
    s.set_cr(0xABCD_EF51);
    s.set_vr(1, a);
    s.set_vr(2, b);
    s.set_vr(3, u128::MAX);
    let insn = PpuInstruction::Vx {
        op,
        rc: false,
        vt: 3,
        va: 1,
        vb: 2,
    };
    assert_eq!(
        exec_no_mem(&insn, &mut s),
        ExecuteVerdict::Continue,
        "{op:?}"
    );
    assert_eq!((s.vscr() & VSCR_SAT, s.cr()), (0, 0xABCD_EF51), "{op:?}");
    s.vr[3]
}

fn is_nan(x: u32) -> bool {
    x & 0x7F80_0000 == 0x7F80_0000 && x & 0x7F_FFFF != 0
}

/// One slot of `op` from the reference.
///
/// [AltiVec-PEM p:6-72 s:6.2] vmaxfp: the larger; the maximum of +0 and -0 is +0.
/// [AltiVec-PEM p:6-81 s:6.2] vminfp: the smaller; the minimum of +0 and -0 is -0.
/// [AltiVec-PEM p:3-15 s:3.2.5.1] a NaN result is vA's NaN, else vB's, quieted.
/// [AltiVec-PEM p:3-11 s:3.2.1] NJ = 1 flushes a denormal to a zero of its sign.
fn reference(op: VxOp, nj: bool, x: u32, y: u32) -> u32 {
    if is_nan(x) {
        return x | 0x40_0000;
    }
    if is_nan(y) {
        return y | 0x40_0000;
    }
    let flush = |v: u32| {
        if nj && v & 0x7F80_0000 == 0 {
            v & 0x8000_0000
        } else {
            v
        }
    };
    let (x, y) = (flush(x), flush(y));
    let max = op == VxOp::Vmaxfp;
    if (x | y) & 0x7FFF_FFFF == 0 {
        let negative = if max { x & y } else { x | y };
        return negative & 0x8000_0000;
    }
    match (order(x, y, nj), max) {
        (Some(Ordering::Less), true) | (Some(Ordering::Greater), false) => y,
        _ => x,
    }
}

#[test]
fn every_class_pair_matches_the_reference_in_both_modes() {
    let pairs: Vec<(u32, u32)> = classes()
        .iter()
        .flat_map(|&x| classes().into_iter().map(move |y| (x, y)))
        .collect();
    assert_eq!(pairs.len(), 400);
    for op in [VxOp::Vmaxfp, VxOp::Vminfp] {
        for nj in [false, true] {
            for chunk in pairs.chunks(4) {
                let (mut a, mut b) = (0u128, 0u128);
                for (i, &(x, y)) in (0..).zip(chunk) {
                    a = with_lane(a, 32, i, x.into());
                    b = with_lane(b, 32, i, y.into());
                }
                let got = run(op, nj, a, b);
                for (i, &(x, y)) in (0..).zip(chunk) {
                    assert_eq!(
                        lane(got, 32, i) as u32,
                        reference(op, nj, x, y),
                        "{op:?} nj={nj} {x:#010x} {y:#010x}"
                    );
                }
            }
        }
    }
}

/// The page's text, in both orders: the pseudocode would give -0 for
/// vmaxfp(-0, +0) and +0 for vminfp(-0, +0).
#[test]
fn every_order_of_the_signed_zeros() {
    let a = pack_u32x4([0x8000_0000, 0, 0x8000_0000, 0]);
    let b = pack_u32x4([0, 0x8000_0000, 0x8000_0000, 0]);
    for nj in [false, true] {
        assert_eq!(
            run(VxOp::Vmaxfp, nj, a, b),
            pack_u32x4([0, 0, 0x8000_0000, 0])
        );
        assert_eq!(
            run(VxOp::Vminfp, nj, a, b),
            pack_u32x4([0x8000_0000, 0x8000_0000, 0x8000_0000, 0])
        );
    }
}

/// vA's NaN wins over vB's, even when vB's is the signaling one.
#[test]
fn the_nan_result_is_the_first_nan_quieted() {
    let a = pack_u32x4([0x7FC1_1111, 0x3F80_0000, 0x7F82_2222, 0xFFC3_3333]);
    let b = pack_u32x4([0x3F80_0000, 0xFF84_4444, 0x7FC5_5555, 0x7F86_6666]);
    let want = pack_u32x4([0x7FC1_1111, 0xFFC4_4444, 0x7FC2_2222, 0xFFC3_3333]);
    for op in [VxOp::Vmaxfp, VxOp::Vminfp] {
        assert_eq!(run(op, false, a, b), want, "{op:?}");
    }
}

/// NJ = 1 writes the flushed zero, never the denormal's own bits.
#[test]
fn a_denormal_operand_is_written_as_its_flushed_zero_under_nj() {
    let a = pack_u32x4([0x0000_0001, 0x8000_0001, 0x0080_0000, 0x807F_FFFF]);
    let b = pack_u32x4([0x3F80_0000, 0xBF80_0000, 0x007F_FFFF, 0]);
    assert_eq!(
        run(VxOp::Vmaxfp, false, a, b),
        pack_u32x4([0x3F80_0000, 0x8000_0001, 0x0080_0000, 0])
    );
    assert_eq!(
        run(VxOp::Vmaxfp, true, a, b),
        pack_u32x4([0x3F80_0000, 0x8000_0000, 0x0080_0000, 0])
    );
    assert_eq!(
        run(VxOp::Vminfp, false, a, b),
        pack_u32x4([0x0000_0001, 0xBF80_0000, 0x007F_FFFF, 0x807F_FFFF])
    );
    assert_eq!(
        run(VxOp::Vminfp, true, a, b),
        pack_u32x4([0, 0xBF80_0000, 0, 0x8000_0000])
    );
}

#[test]
fn infinities_bound_every_finite_value() {
    let a = pack_u32x4([0x7F80_0000, 0xFF80_0000, 0x7F7F_FFFF, 0xFF7F_FFFF]);
    let b = pack_u32x4([0x7F7F_FFFF, 0xFF7F_FFFF, 0xFF80_0000, 0x7F80_0000]);
    assert_eq!(
        run(VxOp::Vmaxfp, false, a, b),
        pack_u32x4([0x7F80_0000, 0xFF7F_FFFF, 0x7F7F_FFFF, 0x7F80_0000])
    );
    assert_eq!(
        run(VxOp::Vminfp, false, a, b),
        pack_u32x4([0x7F7F_FFFF, 0xFF80_0000, 0xFF80_0000, 0xFF7F_FFFF])
    );
}

#[test]
fn vd_va_and_vb_may_name_one_register() {
    let v = pack_u32x4([0x3F80_0000, 0x8000_0000, 0x7F80_0001, 0x0000_0001]);
    for (op, nj, want) in [
        (
            VxOp::Vmaxfp,
            false,
            [0x3F80_0000, 0x8000_0000, 0x7FC0_0001, 1],
        ),
        (
            VxOp::Vminfp,
            true,
            [0x3F80_0000, 0x8000_0000, 0x7FC0_0001, 0],
        ),
    ] {
        let mut s = PpuState::new();
        s.set_vscr(if nj { VSCR_NJ } else { 0 });
        s.set_vr(1, v);
        let insn = PpuInstruction::Vx {
            op,
            rc: false,
            vt: 1,
            va: 1,
            vb: 1,
        };
        assert_eq!(exec_no_mem(&insn, &mut s), ExecuteVerdict::Continue);
        assert_eq!(s.vr[1], pack_u32x4(want), "{op:?}");
    }
}
