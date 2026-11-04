//! The eight even and odd multiplies against a wide-integer reference
//! written from the PEM pseudocode.

use super::lane_support::{lane, operand_classes, pack_u32x4, with_lane};
use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::VSCR_SAT;

/// Each op, its source element width, whether it reads the odd element
/// of each pair, and whether it reads its elements signed.
const OPS: [(VxOp, u32, bool, bool); 8] = [
    (VxOp::Vmuleub, 8, false, false),
    (VxOp::Vmuloub, 8, true, false),
    (VxOp::Vmulesb, 8, false, true),
    (VxOp::Vmulosb, 8, true, true),
    (VxOp::Vmuleuh, 16, false, false),
    (VxOp::Vmulouh, 16, true, false),
    (VxOp::Vmulesh, 16, false, true),
    (VxOp::Vmulosh, 16, true, true),
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

/// The `2w`-bit product of two `w`-bit elements, computed in 64 bits.
///
/// [AltiVec-PEM p:6-101 s:6.2] vmulesb: each even signed byte of vA times that of vB; the eight 16-bit products go, in order, into the half words of vD.
/// [AltiVec-PEM p:6-105 s:6.2] vmulosb: the same, from the odd bytes.
/// [AltiVec-PEM p:6-108 s:6.2] vmulouh: the four 32-bit products of the odd half words go, in order, into the words of vD.
fn reference(w: u32, signed: bool, a: u64, b: u64) -> u64 {
    let extend = |x: u64| -> i64 {
        if signed {
            ((x << (64 - w)) as i64) >> (64 - w)
        } else {
            x as i64
        }
    };
    ((extend(a) * extend(b)) as u64) & ((1u64 << (2 * w)) - 1)
}

#[test]
fn every_class_pair_matches_the_reference_in_some_lane() {
    for (op, w, odd, signed) in OPS {
        let mask = (1u64 << w) - 1;
        // The element of each pair the op does not read holds this value.
        let (noise_a, noise_b) = (0x5A5A_5A5A & mask, 0xC3C3_C3C3 & mask);
        let classes = operand_classes(w);
        let pairs: Vec<(u64, u64)> = classes
            .iter()
            .flat_map(|&a| classes.iter().map(move |&b| (a, b)))
            .collect();
        let mut checked = 0;
        for chunk in pairs.chunks((64 / w) as usize) {
            let (mut a, mut b) = (0u128, 0u128);
            for (j, &(x, y)) in (0..).zip(chunk) {
                let (read, skip) = if odd {
                    (2 * j + 1, 2 * j)
                } else {
                    (2 * j, 2 * j + 1)
                };
                a = with_lane(with_lane(a, w, read, x), w, skip, noise_a);
                b = with_lane(with_lane(b, w, read, y), w, skip, noise_b);
            }
            let r = vx(&mut PpuState::new(), op, a, b);
            for (j, &(x, y)) in (0..).zip(chunk) {
                assert_eq!(
                    lane(r, 2 * w, j),
                    reference(w, signed, x, y),
                    "{op:?} {x:#x}, {y:#x}"
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 36, "{op:?}");
    }
}

/// [AltiVec-PEM p:2-4 s:2.3.1] Element 0 is the most significant element, so the even elements are the high half of each pair.
#[test]
fn even_and_odd_read_their_own_half_of_each_pair() {
    let a = u128::from_be_bytes(std::array::from_fn(|i| 1 + i as u8));
    let b = u128::from_be_bytes(std::array::from_fn(|i| 17 + i as u8));
    let halfs = |f: fn(u16) -> u16| (0..8u16).fold(0u128, |v, j| (v << 16) | u128::from(f(j)));
    let mut s = PpuState::new();
    assert_eq!(
        vx(&mut s, VxOp::Vmuleub, a, b),
        halfs(|j| (2 * j + 1) * (2 * j + 17))
    );
    assert_eq!(
        vx(&mut s, VxOp::Vmuloub, a, b),
        halfs(|j| (2 * j + 2) * (2 * j + 18))
    );

    let a = pack_u32x4([0x0001_0002, 0x0003_0004, 0x0005_0006, 0x0007_0008]);
    let b = pack_u32x4([0x0009_000A, 0x000B_000C, 0x000D_000E, 0x000F_0010]);
    assert_eq!(
        vx(&mut s, VxOp::Vmuleuh, a, b),
        pack_u32x4([9, 3 * 11, 5 * 13, 7 * 15])
    );
    assert_eq!(
        vx(&mut s, VxOp::Vmulouh, a, b),
        pack_u32x4([2 * 10, 4 * 12, 6 * 14, 8 * 16])
    );
}

#[test]
fn the_extreme_products_fit_their_double_width_element() {
    let mut s = PpuState::new();
    let words = |x: u32| pack_u32x4([x; 4]);
    let halfs = |x: u16| (0..8).fold(0u128, |v, _| (v << 16) | u128::from(x));
    let bytes = |x: u8| u128::from_be_bytes([x; 16]);

    let min_h = words(0x8000_8000);
    assert_eq!(vx(&mut s, VxOp::Vmulesh, min_h, min_h), words(0x4000_0000));
    assert_eq!(vx(&mut s, VxOp::Vmulosh, min_h, min_h), words(0x4000_0000));
    assert_eq!(
        vx(&mut s, VxOp::Vmulesb, bytes(0x80), bytes(0x80)),
        halfs(0x4000)
    );
    assert_eq!(
        vx(&mut s, VxOp::Vmuleub, bytes(0xFF), bytes(0xFF)),
        halfs(0xFE01)
    );
    assert_eq!(
        vx(&mut s, VxOp::Vmulosb, bytes(0xFF), bytes(0x01)),
        halfs(0xFFFF)
    );
    assert_eq!(
        vx(
            &mut s,
            VxOp::Vmuleuh,
            words(0xFFFF_FFFF),
            words(0xFFFF_FFFF)
        ),
        words(0xFFFE_0001)
    );
}

/// [AltiVec-PEM p:6-101 s:6.2] vmulesb alters no other register; the pages of the other seven say the same.
#[test]
fn none_of_the_eight_changes_sat() {
    for start_sat in [false, true] {
        for (op, ..) in OPS {
            let mut s = PpuState::new();
            if start_sat {
                s.set_sat();
            }
            vx(
                &mut s,
                op,
                u128::MAX,
                0x8080_8080_8080_8080_8080_8080_8080_8080,
            );
            assert_eq!(s.vscr() & VSCR_SAT != 0, start_sat, "{op:?}");
        }
    }
}
