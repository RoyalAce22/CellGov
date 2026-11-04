//! The eighteen maximum, minimum and average instructions against a
//! wide-integer reference written from the PEM pseudocode.

use super::lane_support::{class_pair_vectors, lane, pack_u32x4};
use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::VSCR_SAT;

#[derive(Clone, Copy, Debug)]
enum Kind {
    Max,
    Min,
    Avg,
}

/// Each op, its element width, whether it reads its elements signed,
/// and what it computes.
const OPS: [(VxOp, u32, bool, Kind); 18] = [
    (VxOp::Vmaxub, 8, false, Kind::Max),
    (VxOp::Vmaxuh, 16, false, Kind::Max),
    (VxOp::Vmaxuw, 32, false, Kind::Max),
    (VxOp::Vmaxsb, 8, true, Kind::Max),
    (VxOp::Vmaxsh, 16, true, Kind::Max),
    (VxOp::Vmaxsw, 32, true, Kind::Max),
    (VxOp::Vminub, 8, false, Kind::Min),
    (VxOp::Vminuh, 16, false, Kind::Min),
    (VxOp::Vminuw, 32, false, Kind::Min),
    (VxOp::Vminsb, 8, true, Kind::Min),
    (VxOp::Vminsh, 16, true, Kind::Min),
    (VxOp::Vminsw, 32, true, Kind::Min),
    (VxOp::Vavgub, 8, false, Kind::Avg),
    (VxOp::Vavguh, 16, false, Kind::Avg),
    (VxOp::Vavguw, 32, false, Kind::Avg),
    (VxOp::Vavgsb, 8, true, Kind::Avg),
    (VxOp::Vavgsh, 16, true, Kind::Avg),
    (VxOp::Vavgsw, 32, true, Kind::Avg),
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

/// One element on `w`-bit elements, computed in 64 bits.
///
/// [AltiVec-PEM p:6-73 s:6.2] vmaxsb: vD takes vA when vA >= vB, compared signed; else vB.
/// [AltiVec-PEM p:6-82 s:6.2] vminsb: vD takes vA when vA < vB, compared signed; else vB.
/// [AltiVec-PEM p:6-43 s:6.2] vavgsb: sign-extend both to 9 bits, add them and 1, keep bits 0:7 of the sum.
fn reference(w: u32, signed: bool, kind: Kind, a: u64, b: u64) -> u64 {
    let mask = (1u64 << w) - 1;
    let extend = |x: u64| -> i64 {
        if signed {
            ((x << (64 - w)) as i64) >> (64 - w)
        } else {
            x as i64
        }
    };
    let (x, y) = (extend(a), extend(b));
    let r = match kind {
        Kind::Max => {
            if x >= y {
                x
            } else {
                y
            }
        }
        Kind::Min => {
            if x < y {
                x
            } else {
                y
            }
        }
        Kind::Avg => (x + y + 1) >> 1,
    };
    (r as u64) & mask
}

#[test]
fn every_class_pair_matches_the_reference_in_some_lane() {
    for (op, w, signed, kind) in OPS {
        let mut checked = 0;
        for (a, b, pairs) in class_pair_vectors(w) {
            let r = vx(&mut PpuState::new(), op, a, b);
            for (i, &(x, y)) in (0..).zip(&pairs) {
                assert_eq!(
                    lane(r, w, i),
                    reference(w, signed, kind, x, y),
                    "{op:?} {x:#x}, {y:#x}"
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 36, "{op:?}");
    }
}

#[test]
fn signedness_decides_the_maximum_of_0x80_and_0x7f() {
    let a = u128::from_be_bytes([0x80; 16]);
    let b = u128::from_be_bytes([0x7F; 16]);
    let mut s = PpuState::new();
    assert_eq!(vx(&mut s, VxOp::Vmaxub, a, b), a);
    assert_eq!(vx(&mut s, VxOp::Vmaxsb, a, b), b);
    assert_eq!(vx(&mut s, VxOp::Vminub, a, b), b);
    assert_eq!(vx(&mut s, VxOp::Vminsb, a, b), a);
}

#[test]
fn the_byte_averages_round_up_in_nine_bits() {
    let bytes = |pairs: &[(u8, u8, u8)]| {
        let pick = |f: fn(&(u8, u8, u8)) -> u8| {
            let mut v = [0u8; 16];
            for (slot, p) in v.iter_mut().zip(pairs) {
                *slot = f(p);
            }
            u128::from_be_bytes(v)
        };
        (pick(|p| p.0), pick(|p| p.1), pick(|p| p.2))
    };
    let mut s = PpuState::new();

    let (a, b, d) = bytes(&[
        (255, 255, 255),
        (0, 1, 1),
        (1, 2, 2),
        (254, 255, 255),
        (0, 0, 0),
    ]);
    assert_eq!(vx(&mut s, VxOp::Vavgub, a, b), d);

    // -1 and -2 give -1; -128 and -128 give -128; 127 and 127 give 127;
    // 1 and 2 give 2; -2 and -3 give -2.
    let (a, b, d) = bytes(&[
        (0xFF, 0xFE, 0xFF),
        (0x80, 0x80, 0x80),
        (0x7F, 0x7F, 0x7F),
        (0xFF, 0x00, 0x00),
        (0x80, 0x7F, 0x00),
        (0x01, 0x02, 0x02),
        (0xFE, 0xFD, 0xFE),
    ]);
    assert_eq!(vx(&mut s, VxOp::Vavgsb, a, b), d);
}

#[test]
fn vavguw_keeps_the_carry_of_a_33_bit_sum() {
    let a = pack_u32x4([0xFFFF_FFFF, 0xFFFF_FFFF, 0x8000_0000, 0]);
    let b = pack_u32x4([0xFFFF_FFFF, 0xFFFF_FFFE, 0x8000_0000, 0xFFFF_FFFF]);
    assert_eq!(
        vx(&mut PpuState::new(), VxOp::Vavguw, a, b),
        pack_u32x4([0xFFFF_FFFF, 0xFFFF_FFFF, 0x8000_0000, 0x8000_0000])
    );
}

/// [AltiVec-PEM p:6-76 s:6.2] vmaxub alters no other register; the pages of the other seventeen say the same.
#[test]
fn none_of_the_eighteen_changes_sat() {
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
                0x7F7F_7F7F_7F7F_7F7F_7F7F_7F7F_7F7F_7F7F,
            );
            assert_eq!(s.vscr() & VSCR_SAT != 0, start_sat, "{op:?}");
        }
    }
}
