//! The four single-precision compares, in both record forms and both NJ
//! modes, against an exact rational reference.

use super::lane_support::{lane, pack_u32x4, with_lane};
use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::VSCR_NJ;
use std::cmp::Ordering;

const OPS: [VxOp; 4] = [
    VxOp::Vcmpeqfp,
    VxOp::Vcmpgefp,
    VxOp::Vcmpgtfp,
    VxOp::Vcmpbfp,
];

// Field 6 starts at 0b0101, which no recording compare produces.
const CR_SEED: u32 = 0xABCD_EF51;

/// Runs `op` on vA, vB = `a`, `b` with VSCR[NJ] = `nj` and CR seeded;
/// returns vD and CR.
fn run(op: VxOp, rc: bool, nj: bool, a: u128, b: u128) -> (u128, u32) {
    let mut s = PpuState::new();
    s.set_vscr(if nj { VSCR_NJ } else { 0 });
    s.set_cr(CR_SEED);
    s.set_vr(1, a);
    s.set_vr(2, b);
    s.set_vr(3, u128::MAX);
    let insn = PpuInstruction::Vx {
        op,
        rc,
        vt: 3,
        va: 1,
        vb: 2,
    };
    assert_eq!(
        exec_no_mem(&insn, &mut s),
        ExecuteVerdict::Continue,
        "{op:?}"
    );
    (s.vr[3], s.cr())
}

/// A binary32 word's value: `None` for a NaN, else its sign and its
/// magnitude as `significand * 2^exponent`; a denormal is zero when `nj`.
///
/// [AltiVec-PEM p:3-15 s:3.2.5] the binary32 encodings.
fn decode(bits: u32, nj: bool) -> Option<(bool, u64, i32)> {
    let negative = bits >> 31 == 1;
    let exponent = (bits >> 23 & 0xFF) as i32;
    let fraction = u64::from(bits & 0x7F_FFFF);
    match exponent {
        0xFF if fraction != 0 => None,
        // An infinity: larger than every finite magnitude.
        0xFF => Some((negative, 1, 1000)),
        0 if nj => Some((negative, 0, 0)),
        0 => Some((negative, fraction, -149)),
        e => Some((negative, fraction | 1 << 23, e - 150)),
    }
}

/// The order of two magnitudes `m * 2^e`.
fn magnitude_order((m1, e1): (u64, i32), (m2, e2): (u64, i32)) -> Ordering {
    match (m1 == 0, m2 == 0) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        _ => {}
    }
    let top = |m: u64, e: i32| e + 63 - m.leading_zeros() as i32;
    match top(m1, e1).cmp(&top(m2, e2)) {
        Ordering::Equal => {
            let low = e1.min(e2);
            (m1 << (e1 - low)).cmp(&(m2 << (e2 - low)))
        }
        other => other,
    }
}

/// The order of two binary32 words, or `None` when either is a NaN.
pub(super) fn order(x: u32, y: u32, nj: bool) -> Option<Ordering> {
    let (sx, mx, ex) = decode(x, nj)?;
    let (sy, my, ey) = decode(y, nj)?;
    let signed = |negative: bool, ord: Ordering| if negative { ord.reverse() } else { ord };
    Some(match (sx && mx != 0, sy && my != 0) {
        (false, false) => magnitude_order((mx, ex), (my, ey)),
        (true, true) => signed(true, magnitude_order((mx, ex), (my, ey))),
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
    })
}

/// One slot of `op`'s result from the reference.
///
/// [AltiVec-PEM p:6-53 s:6.2] vcmpeqfp, [AltiVec-PEM p:6-57 s:6.2] vcmpgefp, [AltiVec-PEM p:6-58 s:6.2] vcmpgtfp: all ones when the relation holds, 0 when not or unordered.
/// [AltiVec-PEM p:6-51 s:6.2] vcmpbfp: bit 0 = !(vA <= vB), bit 1 = !(vA >= -vB).
fn reference(op: VxOp, nj: bool, x: u32, y: u32) -> u32 {
    let holds = |f: fn(Ordering) -> bool| order(x, y, nj).is_some_and(f);
    match op {
        VxOp::Vcmpeqfp => u32::MAX * u32::from(holds(Ordering::is_eq)),
        VxOp::Vcmpgefp => u32::MAX * u32::from(holds(Ordering::is_ge)),
        VxOp::Vcmpgtfp => u32::MAX * u32::from(holds(Ordering::is_gt)),
        VxOp::Vcmpbfp => {
            let le = holds(Ordering::is_le);
            let ge = order(x, y ^ 0x8000_0000, nj).is_some_and(Ordering::is_ge);
            u32::from(!le) << 31 | u32::from(!ge) << 30
        }
        _ => unreachable!("{op:?} is no float compare"),
    }
}

/// Binary32 classes in both signs: zero, the smallest and largest
/// denormal, the smallest normal, 1.0, 1.5, the largest finite, infinity,
/// a QNaN and an SNaN.
pub(super) fn classes() -> Vec<u32> {
    [
        0x0000_0000,
        0x0000_0001,
        0x007F_FFFF,
        0x0080_0000,
        0x3F80_0000,
        0x3FC0_0000,
        0x7F7F_FFFF,
        0x7F80_0000,
        0x7FC0_0001,
        0x7F80_0001,
    ]
    .into_iter()
    .flat_map(|x| [x, x | 0x8000_0000])
    .collect()
}

#[test]
fn every_class_pair_compares_like_the_reference_in_both_modes() {
    let pairs: Vec<(u32, u32)> = classes()
        .iter()
        .flat_map(|&x| classes().into_iter().map(move |y| (x, y)))
        .collect();
    assert_eq!(pairs.len(), 400);
    for op in OPS {
        for nj in [false, true] {
            for chunk in pairs.chunks(4) {
                let (mut a, mut b) = (0u128, 0u128);
                for (i, &(x, y)) in (0..).zip(chunk) {
                    a = with_lane(a, 32, i, x.into());
                    b = with_lane(b, 32, i, y.into());
                }
                let (got, _) = run(op, false, nj, a, b);
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

#[test]
fn plus_and_minus_zero_compare_equal() {
    let (pz, nz) = (pack_u32x4([0; 4]), pack_u32x4([0x8000_0000; 4]));
    for nj in [false, true] {
        assert_eq!(run(VxOp::Vcmpeqfp, false, nj, pz, nz).0, u128::MAX);
        assert_eq!(run(VxOp::Vcmpgefp, false, nj, nz, pz).0, u128::MAX);
        assert_eq!(run(VxOp::Vcmpgtfp, false, nj, pz, nz).0, 0);
        assert_eq!(run(VxOp::Vcmpbfp, false, nj, pz, nz).0, 0);
    }
}

/// [AltiVec-PEM p:3-13 s:3.2.4.1] a NaN gives 0 to the three relational
/// compares and 0xC000_0000 to vcmpbfp.
#[test]
fn a_nan_in_either_operand_is_unordered() {
    let one = 0x3F80_0000;
    let nans = [
        (0x7FC0_0000, one),
        (one, 0xFF80_0001),
        (0x7F80_0001, 0x7FC0_0000),
    ];
    for (x, y) in nans {
        let (a, b) = (pack_u32x4([x; 4]), pack_u32x4([y; 4]));
        for op in [VxOp::Vcmpeqfp, VxOp::Vcmpgefp, VxOp::Vcmpgtfp] {
            assert_eq!(run(op, false, false, a, b).0, 0, "{op:?} {x:#x} {y:#x}");
        }
        assert_eq!(
            run(VxOp::Vcmpbfp, false, false, a, b).0,
            pack_u32x4([0xC000_0000; 4])
        );
    }
}

/// [AltiVec-PEM p:6-51 s:6.2] a negative vB puts every vA out of bounds.
#[test]
fn vcmpbfp_bounds_by_the_magnitude_of_vb() {
    let a = pack_u32x4([0, 0x3F80_0000, 0xBF80_0000, 0x4000_0000]);
    let slots = |b: u32| {
        let (r, _) = run(VxOp::Vcmpbfp, false, false, a, pack_u32x4([b; 4]));
        [0, 1, 2, 3].map(|i| lane(r, 32, i) as u32)
    };
    // vB = -1: the bounds [1, -1] are empty, so every slot is out.
    assert_eq!(
        slots(0xBF80_0000),
        [0xC000_0000, 0x8000_0000, 0x4000_0000, 0x8000_0000]
    );
    assert_eq!(
        slots(0x8000_0000),
        [0, 0x8000_0000, 0x4000_0000, 0x8000_0000]
    );
    assert_eq!(slots(0x7F80_0000), [0; 4]);
    assert_eq!(slots(0x3F80_0000), [0, 0, 0, 0x8000_0000]);
}

/// [AltiVec-PEM p:3-11 s:3.2.1] NJ = 1 flushes a denormal operand to zero
/// before the compare.
#[test]
fn a_denormal_equals_zero_only_under_nj() {
    let (d, z) = (pack_u32x4([0x0000_0001; 4]), pack_u32x4([0; 4]));
    assert_eq!(run(VxOp::Vcmpeqfp, false, false, d, z).0, 0);
    assert_eq!(run(VxOp::Vcmpeqfp, false, true, d, z).0, u128::MAX);
    assert_eq!(run(VxOp::Vcmpgtfp, false, false, d, z).0, u128::MAX);
    assert_eq!(run(VxOp::Vcmpgtfp, false, true, d, z).0, 0);
}

/// [AltiVec-PEM p:6-53 s:6.2] the relational record forms set CR6 to
/// all_true || 0 || all_false || 0; [AltiVec-PEM p:6-51 s:6.2]
/// vcmpbfp. sets CR6 to 0b00 || all_within_bounds || 0.
#[test]
fn each_record_form_sets_cr6_and_no_other_field() {
    let one = 0x3F80_0000;
    let a = pack_u32x4([one, one, 0x4000_0000, 0]);
    let cases: [(VxOp, u128, u32); 9] = [
        (VxOp::Vcmpeqfp, a, 0b1000),
        (VxOp::Vcmpeqfp, pack_u32x4([0x4040_0000; 4]), 0b0010),
        (VxOp::Vcmpeqfp, pack_u32x4([one; 4]), 0),
        (VxOp::Vcmpgefp, pack_u32x4([0xFF80_0000; 4]), 0b1000),
        (VxOp::Vcmpgefp, pack_u32x4([0x7F80_0000; 4]), 0b0010),
        (VxOp::Vcmpgtfp, pack_u32x4([one; 4]), 0),
        (VxOp::Vcmpbfp, pack_u32x4([0x4000_0000; 4]), 0b0010),
        (VxOp::Vcmpbfp, pack_u32x4([one; 4]), 0),
        (VxOp::Vcmpbfp, pack_u32x4([0x7FC0_0000; 4]), 0),
    ];
    for (op, b, cr6) in cases {
        let (_, cr) = run(op, true, false, a, b);
        assert_eq!(cr, CR_SEED & !0xF0 | cr6 << 4, "{op:?} {b:#x}");
        let (_, cr) = run(op, false, false, a, b);
        assert_eq!(cr, CR_SEED, "{op:?} Rc=0");
    }
}
