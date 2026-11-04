//! The six unpack instructions against a reference written from the PEM
//! pseudocode.

use super::lane_support::{lane, pack_u32x4};
use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::VSCR_SAT;

/// Each op, its source element width, whether it reads the high half of
/// vB, and whether its elements are pixels.
const OPS: [(VxOp, u32, bool, bool); 6] = [
    (VxOp::Vupkhsb, 8, true, false),
    (VxOp::Vupklsb, 8, false, false),
    (VxOp::Vupkhsh, 16, true, false),
    (VxOp::Vupklsh, 16, false, false),
    (VxOp::Vupkhpx, 16, true, true),
    (VxOp::Vupklpx, 16, false, true),
];

/// Runs `op` on vB with `a` in the reserved vA register.
fn unpack(op: VxOp, a: u128, b: u128) -> u128 {
    let mut s = PpuState::new();
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
    s.vr[3]
}

/// One source element widened to `2w` bits.
///
/// [AltiVec-PEM p:6-172 s:6.2] vupkhsb: each byte sign-extends to 16 bits.
/// [AltiVec-PEM p:6-171 s:6.2] vupkhpx: SignExtend(bit 0, 8) || ZeroExtend(bits 1:5, 8) || ZeroExtend(bits 6:10, 8) || ZeroExtend(bits 11:15, 8).
fn reference(w: u32, pixel: bool, x: u64) -> u64 {
    if pixel {
        let bits = |hi: u32, lo: u32| (x >> (15 - lo)) & ((1 << (lo - hi + 1)) - 1);
        let alpha = if bits(0, 0) == 1 { 0xFF } else { 0 };
        (alpha << 24) | (bits(1, 5) << 16) | (bits(6, 10) << 8) | bits(11, 15)
    } else {
        let signed = ((x << (64 - w)) as i64) >> (64 - w);
        (signed as u64) & ((1u64 << (2 * w)) - 1)
    }
}

/// Source classes for `w`-bit elements: 0, 1, all ones, the sign bit only,
/// the largest positive value, and arbitrary values.
fn classes(w: u32) -> Vec<u64> {
    let mask = (1u64 << w) - 1;
    let sign = 1u64 << (w - 1);
    vec![
        0,
        1,
        mask,
        sign,
        sign - 1,
        0x9E37 & mask,
        0x79B9 & mask,
        0x5AC3 & mask,
    ]
}

#[test]
fn every_class_in_every_element_matches_the_reference() {
    for (op, w, high, pixel) in OPS {
        let per_half = (64 / w) as usize;
        let classes = classes(w);
        let mut checked = 0;
        // Rotate the classes through the elements, so each class visits
        // every element position.
        for start in 0..classes.len() {
            let elems: Vec<u64> = (0..per_half)
                .map(|j| classes[(start + j) % classes.len()])
                .collect();
            let half = elems.iter().fold(0u64, |h, &e| (h << w) | e);
            // The other half holds the complement, so reading it fails.
            let b = if high {
                (u128::from(half) << 64) | u128::from(!half)
            } else {
                (u128::from(!half) << 64) | u128::from(half)
            };
            let r = unpack(op, 0, b);
            for (j, &e) in (0..).zip(&elems) {
                assert_eq!(lane(r, 2 * w, j), reference(w, pixel, e), "{op:?} {e:#x}");
                checked += 1;
            }
        }
        assert_eq!(checked, classes.len() * per_half, "{op:?}");
    }
}

#[test]
fn high_and_low_read_their_own_half() {
    let b = u128::from_be_bytes(std::array::from_fn(|i| 0x10 * (i as u8 % 8) + i as u8));
    let halfs = |bytes: [u8; 8]| bytes.iter().fold(0u128, |v, &x| (v << 16) | u128::from(x));
    assert_eq!(
        unpack(VxOp::Vupkhsb, 0, b),
        halfs([0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77])
    );
    assert_eq!(
        unpack(VxOp::Vupklsb, 0, b),
        halfs([0x08, 0x19, 0x2A, 0x3B, 0x4C, 0x5D, 0x6E, 0x7F])
    );
    let b = pack_u32x4([0x0001_0002, 0x0003_0004, 0x0005_0006, 0x0007_0008]);
    assert_eq!(unpack(VxOp::Vupkhsh, 0, b), pack_u32x4([1, 2, 3, 4]));
    assert_eq!(unpack(VxOp::Vupklsh, 0, b), pack_u32x4([5, 6, 7, 8]));
}

#[test]
fn a_pixel_spreads_its_alpha_bit_and_three_channels_over_four_bytes() {
    let pixels = |p: [u16; 4]| {
        (u128::from(p[0]) << 48
            | u128::from(p[1]) << 32
            | u128::from(p[2]) << 16
            | u128::from(p[3]))
            << 64
    };
    // Alpha only, all channels full, then a different marker bit in each
    // channel.
    let b = pixels([0x8000, 0x7FFF, 0x4000 | 0x0100 | 0x0004, 0x0001]);
    assert_eq!(
        unpack(VxOp::Vupkhpx, 0, b),
        pack_u32x4([0xFF00_0000, 0x001F_1F1F, 0x0010_0804, 0x0000_0001])
    );
    assert_eq!(
        unpack(VxOp::Vupklpx, 0, b >> 64),
        unpack(VxOp::Vupkhpx, 0, b)
    );
}

#[test]
fn the_reserved_va_register_does_not_change_the_result() {
    let b = 0x8001_7FFF_FF80_0123_4567_89AB_CDEF_F00F;
    for (op, ..) in OPS {
        assert_eq!(unpack(op, u128::MAX, b), unpack(op, 0, b), "{op:?}");
    }
}

/// [AltiVec-PEM p:6-172 s:6.2] vupkhsb alters no other register; the pages of the other five say the same.
#[test]
fn none_of_the_six_changes_sat() {
    for start_sat in [false, true] {
        for (op, ..) in OPS {
            let mut s = PpuState::new();
            if start_sat {
                s.set_sat();
            }
            s.set_vr(2, u128::MAX);
            let insn = PpuInstruction::Vx {
                op,
                rc: false,
                vt: 3,
                va: 1,
                vb: 2,
            };
            assert_eq!(exec_no_mem(&insn, &mut s), ExecuteVerdict::Continue);
            assert_eq!(s.vscr() & VSCR_SAT != 0, start_sat, "{op:?}");
        }
    }
}
