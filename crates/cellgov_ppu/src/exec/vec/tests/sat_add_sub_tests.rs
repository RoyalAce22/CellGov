//! The eleven saturating adds and subtracts and the SAT they set, against
//! a wide-integer reference written from the PEM pseudocode.

use super::lane_support::{lane, with_lane};
use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::VSCR_SAT;

/// Each op, its element width, whether it reads its elements signed, and
/// whether it subtracts.
const OPS: [(VxOp, u32, bool, bool); 12] = [
    (VxOp::Vaddubs, 8, false, false),
    (VxOp::Vadduhs, 16, false, false),
    (VxOp::Vadduws, 32, false, false),
    (VxOp::Vaddsbs, 8, true, false),
    (VxOp::Vaddshs, 16, true, false),
    (VxOp::Vaddsws, 32, true, false),
    (VxOp::Vsububs, 8, false, true),
    (VxOp::Vsubuhs, 16, false, true),
    (VxOp::Vsubuws, 32, false, true),
    (VxOp::Vsubsbs, 8, true, true),
    (VxOp::Vsubshs, 16, true, true),
    (VxOp::Vsubsws, 32, true, true),
];

/// Runs `op` from a state whose SAT is `sat`; returns vD and SAT after.
fn run(op: VxOp, sat: bool, a: u128, b: u128) -> (u128, bool) {
    let mut s = PpuState::new();
    if sat {
        s.set_sat();
    }
    s.set_vr(1, a);
    s.set_vr(2, b);
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
    (s.vr[3], s.vscr() & VSCR_SAT != 0)
}

/// One element and whether it clamps, computed in 64 bits.
///
/// [AltiVec-PEM p:6-32 s:6.2] vaddsbs: sign-extend both to 9 bits, add, saturate to [-2^7, 2^7 - 1] and set SAT when it saturates.
/// [AltiVec-PEM p:6-165 s:6.2] vsubuws: zero-extend both to 33 bits, subtract, saturate below 0 to 0.
/// [AltiVec-PEM p:4-4 s:4.2.1.1] A result equal to a bound without clamping is not saturation.
fn reference(w: u32, signed: bool, sub: bool, a: u64, b: u64) -> (u64, bool) {
    let mask = (1u64 << w) - 1;
    let extend = |x: u64| -> i64 {
        if signed {
            ((x << (64 - w)) as i64) >> (64 - w)
        } else {
            x as i64
        }
    };
    let v = if sub {
        extend(a) - extend(b)
    } else {
        extend(a) + extend(b)
    };
    let (min, max) = if signed {
        (-(1i64 << (w - 1)), (1i64 << (w - 1)) - 1)
    } else {
        (0, (1i64 << w) - 1)
    };
    let clamped = v.clamp(min, max);
    ((clamped as u64) & mask, clamped != v)
}

/// Operand classes for `w`-bit elements: 0, 1, all ones, the signed max
/// and min, one inside each signed bound, and a fixed arbitrary value.
fn classes(w: u32) -> [u64; 8] {
    let mask = (1u64 << w) - 1;
    let sign = 1u64 << (w - 1);
    [
        0,
        1,
        mask,
        sign - 1,
        sign,
        sign - 2,
        sign + 1,
        0x9E37_79B9 & mask,
    ]
}

#[test]
fn every_class_pair_matches_the_reference_and_its_sat() {
    for (op, w, signed, sub) in OPS {
        let classes = classes(w);
        let pairs: Vec<(u64, u64)> = classes
            .iter()
            .flat_map(|&a| classes.iter().map(move |&b| (a, b)))
            .collect();
        let mut checked = 0;
        for chunk in pairs.chunks((128 / w) as usize) {
            let (mut a, mut b) = (0u128, 0u128);
            for (i, &(x, y)) in (0..).zip(chunk) {
                a = with_lane(a, w, i, x);
                b = with_lane(b, w, i, y);
            }
            let (r, sat) = run(op, false, a, b);
            let mut any = false;
            for (i, &(x, y)) in (0..).zip(chunk) {
                let (want, clamps) = reference(w, signed, sub, x, y);
                assert_eq!(lane(r, w, i), want, "{op:?} {x:#x}, {y:#x}");
                any |= clamps;
                checked += 1;
            }
            assert_eq!(sat, any, "{op:?} SAT over {chunk:x?}");
        }
        assert_eq!(checked, 64, "{op:?}");
    }
}

fn bytes(x: u8) -> u128 {
    u128::from_be_bytes([x; 16])
}

#[test]
fn a_result_on_a_bound_leaves_sat_clear() {
    assert_eq!(
        run(VxOp::Vaddubs, false, bytes(1), bytes(254)),
        (bytes(255), false)
    );
    assert_eq!(
        run(VxOp::Vaddsbs, false, bytes(127), bytes(0)),
        (bytes(127), false)
    );
    assert_eq!(
        run(VxOp::Vsubsbs, false, bytes(0x80), bytes(0)),
        (bytes(0x80), false)
    );
    assert_eq!(run(VxOp::Vsububs, false, bytes(5), bytes(5)), (0, false));
}

#[test]
fn a_clamp_sets_sat() {
    assert_eq!(
        run(VxOp::Vaddubs, false, bytes(255), bytes(1)),
        (bytes(255), true)
    );
    assert_eq!(
        run(VxOp::Vaddsbs, false, bytes(127), bytes(1)),
        (bytes(127), true)
    );
    assert_eq!(
        run(VxOp::Vsubsbs, false, bytes(0x80), bytes(1)),
        (bytes(0x80), true)
    );
    // 0 - (-128) is 128, one past the signed max.
    assert_eq!(
        run(VxOp::Vsubsbs, false, 0, bytes(0x80)),
        (bytes(0x7F), true)
    );
}

#[test]
fn one_clamping_lane_sets_sat_wherever_it_is() {
    for lane_index in 0..16 {
        let mut a = [1u8; 16];
        a[lane_index] = 255;
        let (r, sat) = run(VxOp::Vaddubs, false, u128::from_be_bytes(a), bytes(1));
        let mut want = [2u8; 16];
        want[lane_index] = 255;
        assert_eq!(r, u128::from_be_bytes(want), "lane {lane_index}");
        assert!(sat, "lane {lane_index}");
    }
}

/// [AltiVec-PEM p:2-5 s:2.3.2] SAT stays set until software clears it.
#[test]
fn sat_set_before_stays_set_after_an_op_that_does_not_clamp() {
    for (op, ..) in OPS {
        let (_, sat) = run(op, true, bytes(2), bytes(1));
        assert!(sat, "{op:?}");
    }
}
