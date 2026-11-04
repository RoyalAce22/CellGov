//! The element rotates and the byte and half-word shifts against a
//! reference written from the PEM pseudocode.

use super::lane_support::{lane, with_lane};
use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::VSCR_SAT;

#[derive(Clone, Copy, Debug)]
enum Kind {
    Rotl,
    Shl,
    Shr,
}

const OPS: [(VxOp, u32, Kind); 7] = [
    (VxOp::Vrlb, 8, Kind::Rotl),
    (VxOp::Vrlh, 16, Kind::Rotl),
    (VxOp::Vrlw, 32, Kind::Rotl),
    (VxOp::Vslb, 8, Kind::Shl),
    (VxOp::Vslh, 16, Kind::Shl),
    (VxOp::Vsrb, 8, Kind::Shr),
    (VxOp::Vsrh, 16, Kind::Shr),
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

/// One `w`-bit element of vA moved by the vB element `count`.
///
/// [AltiVec-PEM p:6-129 s:6.2] vrlb: the count is the low 3 bits of the vB byte; the vA byte rotates left by it.
/// [AltiVec-PEM p:6-135 s:6.2] vslb: shift left by that count; bits out of bit 0 are lost, zeros fill on the right.
/// [AltiVec-PEM p:6-151 s:6.2] vsrb: shift right by that count; bits out of bit 7 are lost, zeros fill on the left.
fn reference(w: u32, kind: Kind, x: u64, count: u64) -> u64 {
    let mask = (1u64 << w) - 1;
    let s = count % u64::from(w);
    match kind {
        Kind::Rotl => ((x << s) | (x >> ((u64::from(w) - s) % u64::from(w)))) & mask,
        Kind::Shl => (x << s) & mask,
        Kind::Shr => x >> s,
    }
}

/// vA classes for `w`-bit elements: 0, 1, all ones, the sign bit only and
/// a fixed arbitrary value.
fn classes(w: u32) -> [u64; 5] {
    let mask = (1u64 << w) - 1;
    [0, 1, mask, 1 << (w - 1), 0x9E37_79B9 & mask]
}

#[test]
fn every_count_with_and_without_high_bits_matches_the_reference() {
    for (op, w, kind) in OPS {
        let mask = (1u64 << w) - 1;
        let high = mask & !u64::from(w - 1);
        let counts: Vec<u64> = (0..u64::from(w)).flat_map(|c| [c, c | high]).collect();
        let pairs: Vec<(u64, u64)> = classes(w)
            .iter()
            .flat_map(|&x| counts.iter().map(move |&c| (x, c)))
            .collect();
        let mut checked = 0;
        for chunk in pairs.chunks((128 / w) as usize) {
            let (mut a, mut b) = (0u128, 0u128);
            for (i, &(x, c)) in (0..).zip(chunk) {
                a = with_lane(a, w, i, x);
                b = with_lane(b, w, i, c);
            }
            let r = vx(&mut PpuState::new(), op, a, b);
            for (i, &(x, c)) in (0..).zip(chunk) {
                assert_eq!(
                    lane(r, w, i),
                    reference(w, kind, x, c),
                    "{op:?} {x:#x} by {c:#x}"
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 5 * 2 * w as usize, "{op:?}");
    }
}

/// [AltiVec-PEM p:2-4 s:2.3.1] Element 0 is the most significant element.
/// [AltiVec-PEM p:6-129 s:6.2] vrlb: each vA element takes the count in the corresponding vB element.
#[test]
fn each_element_takes_the_count_in_its_own_lane() {
    let mut s = PpuState::new();
    let ones = u128::from_be_bytes([1; 16]);
    let counts = u128::from_be_bytes(std::array::from_fn(|i| i as u8));
    let shifted = u128::from_be_bytes(std::array::from_fn(|i| 1u8 << (i % 8)));
    assert_eq!(vx(&mut s, VxOp::Vslb, ones, counts), shifted);
    assert_eq!(vx(&mut s, VxOp::Vrlb, ones, counts), shifted);

    let halfs = |f: fn(u16) -> u16| (0..8u16).fold(0u128, |v, j| (v << 16) | u128::from(f(j)));
    let top = halfs(|_| 0x8000);
    assert_eq!(
        vx(&mut s, VxOp::Vsrh, top, halfs(|j| 2 * j + 1)),
        halfs(|j| 0x8000 >> (2 * j + 1))
    );
}

#[test]
fn a_rotate_by_zero_is_the_identity_and_differs_from_a_shift_by_width_less_one() {
    let mut s = PpuState::new();
    let v = 0x8001_7FFE_0123_4567_89AB_CDEF_F00F_0FF0;
    for op in [VxOp::Vrlb, VxOp::Vrlh, VxOp::Vrlw] {
        assert_eq!(vx(&mut s, op, v, 0), v, "{op:?}");
    }
    let sevens = u128::from_be_bytes([7; 16]);
    let threes = u128::from_be_bytes([0x03; 16]);
    assert_eq!(
        vx(&mut s, VxOp::Vrlb, threes, sevens),
        u128::from_be_bytes([0x81; 16])
    );
    assert_eq!(
        vx(&mut s, VxOp::Vslb, threes, sevens),
        u128::from_be_bytes([0x80; 16])
    );
}

/// [AltiVec-PEM p:6-129 s:6.2] vrlb alters no other register; the pages of the other six say the same.
#[test]
fn none_of_the_seven_changes_sat() {
    for start_sat in [false, true] {
        for (op, ..) in OPS {
            let mut s = PpuState::new();
            if start_sat {
                s.set_sat();
            }
            vx(&mut s, op, u128::MAX, u128::from_be_bytes([0x07; 16]));
            assert_eq!(s.vscr() & VSCR_SAT != 0, start_sat, "{op:?}");
        }
    }
}
