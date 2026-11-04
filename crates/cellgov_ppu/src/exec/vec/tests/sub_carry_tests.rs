//! `vaddcuw`, `vsububm`, `vsubuhm`, `vsubuwm` and `vsubcuw` against a
//! wide-integer reference written from the PEM pseudocode.

use super::lane_support::pack_u32x4;
use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::VSCR_SAT;

const OPS: [(VxOp, u32); 5] = [
    (VxOp::Vaddcuw, 32),
    (VxOp::Vsububm, 8),
    (VxOp::Vsubuhm, 16),
    (VxOp::Vsubuwm, 32),
    (VxOp::Vsubcuw, 32),
];

fn vx(s: &mut PpuState, op: VxOp, a: u128, b: u128) -> u128 {
    s.set_vr(1, a);
    s.set_vr(2, b);
    let insn = PpuInstruction::Vx {
        op,
        rc: false,
        vt: 3,
        va: 1,
        vb: 2,
    };
    assert_eq!(exec_no_mem(&insn, s), ExecuteVerdict::Continue, "{op:?}");
    s.vr[3]
}

/// One element of `op` on `w`-bit elements, computed in 64 bits.
///
/// [AltiVec-PEM p:6-30 s:6.2] vaddcuw: temp0:32 = ZeroExtend(vA,33) + ZeroExtend(vB,33); vD = ZeroExtend(temp0, 32).
/// [AltiVec-PEM p:6-155 s:6.2] vsubcuw: vA plus the complement of vB plus 1 in 33 bits; vD = ZeroExtend(temp0, 32).
/// [AltiVec-PEM p:6-160 s:6.2] The modulo subtracts add the two's complement of the vB element to the vA element.
fn reference(op: VxOp, w: u32, a: u64, b: u64) -> u64 {
    let mask = (1u64 << w) - 1;
    match op {
        VxOp::Vaddcuw => (a + b) >> 32,
        VxOp::Vsubcuw => (a + (b ^ mask) + 1) >> 32,
        _ => (a + (b ^ mask) + 1) & mask,
    }
}

fn lane(v: u128, w: u32, i: u32) -> u64 {
    let lanes = 128 / w;
    ((v >> ((lanes - 1 - i) * w)) as u64) & ((1u64 << w) - 1)
}

fn with_lane(v: u128, w: u32, i: u32, x: u64) -> u128 {
    let lanes = 128 / w;
    v | (u128::from(x) << ((lanes - 1 - i) * w))
}

/// Operand classes for `w`-bit elements: 0, 1, unsigned max (all ones),
/// the two sides of the sign boundary, and a fixed arbitrary value.
fn classes(w: u32) -> [u64; 6] {
    let mask = (1u64 << w) - 1;
    let sign = 1u64 << (w - 1);
    [0, 1, mask, sign - 1, sign, 0x9E37_79B9 & mask]
}

#[test]
fn every_class_pair_matches_the_reference_in_some_lane() {
    for (op, w) in OPS {
        let classes = classes(w);
        let pairs: Vec<(u64, u64)> = classes
            .iter()
            .flat_map(|&a| classes.iter().map(move |&b| (a, b)))
            .collect();
        let lanes = 128 / w;
        let mut checked = 0;
        for chunk in pairs.chunks(lanes as usize) {
            let (mut a, mut b) = (0u128, 0u128);
            for (i, &(x, y)) in (0..).zip(chunk) {
                a = with_lane(a, w, i, x);
                b = with_lane(b, w, i, y);
            }
            let r = vx(&mut PpuState::new(), op, a, b);
            for (i, &(x, y)) in (0..).zip(chunk) {
                assert_eq!(
                    lane(r, w, i),
                    reference(op, w, x, y),
                    "{op:?} {x:#x}, {y:#x}"
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 36, "{op:?}");
    }
}

#[test]
fn vaddcuw_is_the_carry_out_of_each_word_sum() {
    let a = pack_u32x4([0xFFFF_FFFF, 0xFFFF_FFFF, 0x8000_0000, 0x7FFF_FFFF]);
    let b = pack_u32x4([1, 0, 0x8000_0000, 0x8000_0000]);
    assert_eq!(
        vx(&mut PpuState::new(), VxOp::Vaddcuw, a, b),
        pack_u32x4([1, 0, 1, 0])
    );
}

#[test]
fn vsubcuw_is_one_when_va_is_not_below_vb() {
    let a = pack_u32x4([7, 6, 0, 0xFFFF_FFFF]);
    let b = pack_u32x4([7, 7, 0, 0]);
    assert_eq!(
        vx(&mut PpuState::new(), VxOp::Vsubcuw, a, b),
        pack_u32x4([1, 0, 1, 1])
    );
}

/// [AltiVec-PEM p:2-4 s:2.3.1] Element 0 is the most significant element.
#[test]
fn the_modulo_subtracts_keep_each_element_in_its_lane() {
    let mut s = PpuState::new();
    let bytes_a: [u8; 16] = std::array::from_fn(|i| 0x40 + 3 * i as u8);
    let bytes_b: [u8; 16] = std::array::from_fn(|i| i as u8);
    let bytes_d: [u8; 16] = std::array::from_fn(|i| 0x40 + 2 * i as u8);
    assert_eq!(
        vx(
            &mut s,
            VxOp::Vsububm,
            u128::from_be_bytes(bytes_a),
            u128::from_be_bytes(bytes_b)
        ),
        u128::from_be_bytes(bytes_d)
    );

    let halfs = |h: [u16; 8]| h.iter().fold(0u128, |v, &x| (v << 16) | u128::from(x));
    assert_eq!(
        vx(
            &mut s,
            VxOp::Vsubuhm,
            halfs([800, 700, 600, 500, 400, 300, 200, 100]),
            halfs([8, 7, 6, 5, 4, 3, 2, 1])
        ),
        halfs([792, 693, 594, 495, 396, 297, 198, 99])
    );

    assert_eq!(
        vx(
            &mut s,
            VxOp::Vsubuwm,
            pack_u32x4([40_000, 30_000, 20_000, 10_000]),
            pack_u32x4([4, 3, 2, 1])
        ),
        pack_u32x4([39_996, 29_997, 19_998, 9_999])
    );
}

/// [AltiVec-PEM p:4-4 s:4.2.1.1] Modulo instructions do not set SAT.
#[test]
fn none_of_the_five_changes_sat() {
    for start_sat in [false, true] {
        for (op, _) in OPS {
            let mut s = PpuState::new();
            if start_sat {
                s.set_sat();
            }
            // Every element wraps or borrows: 0 - max, and max + max.
            let (a, b) = match op {
                VxOp::Vaddcuw => (u128::MAX, u128::MAX),
                _ => (0, u128::MAX),
            };
            vx(&mut s, op, a, b);
            assert_eq!(s.vscr() & VSCR_SAT != 0, start_sat, "{op:?}");
        }
    }
}
