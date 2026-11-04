//! The nine pack instructions and the SAT they set, against a
//! per-element reference written from the PEM pages.

use super::lane_support::{class_pair_vectors, lane, operand_classes, pack_u32x4, with_lane};
use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::VSCR_SAT;

const OPS: [VxOp; 9] = [
    VxOp::Vpkuhum,
    VxOp::Vpkuwum,
    VxOp::Vpkuhus,
    VxOp::Vpkuwus,
    VxOp::Vpkshus,
    VxOp::Vpkswus,
    VxOp::Vpkshss,
    VxOp::Vpkswss,
    VxOp::Vpkpx,
];

/// Runs `op` on vD, vA, vB = v3, v1, v2 with vD preset to all ones from
/// a state whose SAT is `sat`; returns vD and SAT after.
fn run(op: VxOp, sat: bool, a: u128, b: u128) -> (u128, bool) {
    let mut s = PpuState::new();
    if sat {
        s.set_sat();
    }
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
    (s.vr[3], s.vscr() & VSCR_SAT != 0)
}

fn sx(x: u64, w: u32) -> i64 {
    ((x << (64 - w)) as i64) >> (64 - w)
}

/// `v` saturated to `[min, max]`, as a `w`-bit field, and whether it
/// saturated.
fn sat(v: i64, min: i64, max: i64, w: u32) -> (u64, bool) {
    let r = v.clamp(min, max);
    ((r as u64) & ((1u64 << w) - 1), r != v)
}

/// The source element width of `op`.
fn width(op: VxOp) -> u32 {
    match op {
        VxOp::Vpkuhum | VxOp::Vpkuhus | VxOp::Vpkshus | VxOp::Vpkshss => 16,
        _ => 32,
    }
}

/// One element of vD from one source element `x`, and whether it
/// saturated.
///
/// [AltiVec-PEM p:6-119 s:6.2] vpkuhum: the low-order byte of the half word.
/// [AltiVec-PEM p:6-121 s:6.2] vpkuwum: the low-order half word of the word.
/// [AltiVec-PEM p:6-120 s:6.2] vpkuhus: the unsigned half word saturated to 2^8 - 1.
/// [AltiVec-PEM p:6-122 s:6.2] vpkuwus: the unsigned word saturated to 2^16 - 1.
/// [AltiVec-PEM p:6-116 s:6.2] vpkshus: the signed half word saturated to [0, 2^8 - 1].
/// [AltiVec-PEM p:6-118 s:6.2] vpkswus: the signed word saturated to [0, 2^16 - 1].
/// [AltiVec-PEM p:6-115 s:6.2] vpkshss: the signed half word saturated to [-2^7, 2^7 - 1].
/// [AltiVec-PEM p:6-117 s:6.2] vpkswss: the signed word saturated to [-2^15, 2^15 - 1].
/// [AltiVec-PEM p:6-113 s:6.2] vpkpx: bit 7 || bits 8:12 || bits 16:20 || bits 24:28 of the word.
fn reference(op: VxOp, x: u64) -> (u64, bool) {
    match op {
        VxOp::Vpkuhum => (x & 0xFF, false),
        VxOp::Vpkuwum => (x & 0xFFFF, false),
        VxOp::Vpkuhus => sat(x as i64, 0, 0xFF, 8),
        VxOp::Vpkuwus => sat(x as i64, 0, 0xFFFF, 16),
        VxOp::Vpkshus => sat(sx(x, 16), 0, 0xFF, 8),
        VxOp::Vpkswus => sat(sx(x, 32), 0, 0xFFFF, 16),
        VxOp::Vpkshss => sat(sx(x, 16), -0x80, 0x7F, 8),
        VxOp::Vpkswss => sat(sx(x, 32), -0x8000, 0x7FFF, 16),
        VxOp::Vpkpx => {
            let bit = |n: u32| (x >> (31 - n)) & 1;
            let field = |from: u32| (from..from + 5).fold(0, |acc, n| acc << 1 | bit(n));
            (
                bit(7) << 15 | field(8) << 10 | field(16) << 5 | field(24),
                false,
            )
        }
        _ => unreachable!("{op:?} is no pack instruction"),
    }
}

/// vD and SAT for `a`, `b` per the reference: vA's elements narrowed
/// into the high half of vD, vB's into the low half.
fn expected(op: VxOp, a: u128, b: u128) -> (u128, bool) {
    let w = width(op);
    let per = 128 / w;
    let mut r = 0u128;
    let mut any = false;
    for (k, v) in (0..).zip([a, b]) {
        for i in 0..per {
            let (n, s) = reference(op, lane(v, w, i));
            r = with_lane(r, w / 2, k * per + i, n);
            any |= s;
        }
    }
    (r, any)
}

#[test]
fn every_class_pair_packs_to_the_reference_and_sets_sat_when_any_element_saturates() {
    for op in OPS {
        let vectors = class_pair_vectors(width(op));
        assert_eq!(
            vectors.len(),
            36usize.div_ceil((128 / width(op)) as usize),
            "{op:?}"
        );
        for (a, b, _) in vectors {
            let (want, want_sat) = expected(op, a, b);
            assert_eq!(
                run(op, false, a, b),
                (want, want_sat),
                "{op:?} {a:#x} {b:#x}"
            );
            assert_eq!(
                run(op, true, a, b),
                (want, true),
                "{op:?} {a:#x} {b:#x} SAT kept"
            );
        }
    }
}

#[test]
fn the_classes_drive_every_saturating_op_both_ways() {
    for op in OPS {
        let outcomes: Vec<bool> = operand_classes(width(op))
            .into_iter()
            .map(|x| reference(op, x).1)
            .collect();
        let saturating = !matches!(op, VxOp::Vpkuhum | VxOp::Vpkuwum | VxOp::Vpkpx);
        assert_eq!(outcomes.contains(&true), saturating, "{op:?}");
        assert!(outcomes.contains(&false), "{op:?}");
    }
}

#[test]
fn va_fills_the_high_half_and_vb_the_low_half_in_element_order() {
    let a = u128::from_be_bytes([0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7, 0, 8]);
    let b = u128::from_be_bytes([0, 9, 0, 10, 0, 11, 0, 12, 0, 13, 0, 14, 0, 15, 0, 16]);
    let want = u128::from_be_bytes([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    assert_eq!(run(VxOp::Vpkuhum, false, a, b), (want, false));
    assert_eq!(run(VxOp::Vpkuhus, false, a, b), (want, false));
    assert_eq!(run(VxOp::Vpkshus, false, a, b), (want, false));
    assert_eq!(run(VxOp::Vpkshss, false, a, b), (want, false));

    let a = pack_u32x4([1, 2, 3, 4]);
    let b = pack_u32x4([5, 6, 7, 8]);
    let want = pack_u32x4([0x0001_0002, 0x0003_0004, 0x0005_0006, 0x0007_0008]);
    for op in [VxOp::Vpkuwum, VxOp::Vpkuwus, VxOp::Vpkswus, VxOp::Vpkswss] {
        assert_eq!(run(op, false, a, b), (want, false), "{op:?}");
    }
}

#[test]
fn a_clamp_in_one_element_alone_sets_sat() {
    let half = |i: u32, x: u64| with_lane(0, 16, i, x);
    let word = |i: u32, x: u64| with_lane(0, 32, i, x);
    // A negative signed half word clamps to an unsigned 0.
    assert_eq!(run(VxOp::Vpkshus, false, half(0, 0xFFFF), 0), (0, true));
    // 0x8000 is an unsigned half word above 2^8 - 1.
    assert_eq!(
        run(VxOp::Vpkuhus, false, 0, half(7, 0x8000)),
        (with_lane(0, 8, 15, 0xFF), true)
    );
    // A value on the bound does not saturate.
    assert_eq!(
        run(VxOp::Vpkswss, false, word(1, 32767), 0),
        (with_lane(0, 16, 1, 0x7FFF), false)
    );
    assert_eq!(
        run(VxOp::Vpkswss, false, 0, word(3, 32768)),
        (with_lane(0, 16, 7, 0x7FFF), true)
    );
    assert_eq!(
        run(
            VxOp::Vpkswss,
            false,
            word(2, (-32769i64 as u64) & 0xFFFF_FFFF),
            0
        ),
        (with_lane(0, 16, 2, 0x8000), true)
    );
    assert_eq!(
        run(VxOp::Vpkshss, false, half(5, 0xFF80), 0),
        (with_lane(0, 8, 5, 0x80), false)
    );
    assert_eq!(
        run(VxOp::Vpkshss, false, half(5, 0xFF7F), 0),
        (with_lane(0, 8, 5, 0x80), true)
    );
    assert_eq!(
        run(VxOp::Vpkuwus, false, 0, word(0, 0x1_0000)),
        (with_lane(0, 16, 4, 0xFFFF), true)
    );
    assert_eq!(
        run(VxOp::Vpkswus, false, word(3, 0x8000_0000), 0),
        (0, true)
    );
}

#[test]
fn vpkpx_keeps_bit_7_and_the_top_five_bits_of_each_low_byte() {
    assert_eq!(
        run(VxOp::Vpkpx, false, u128::MAX, u128::MAX),
        (u128::MAX, false)
    );
    let dropped = pack_u32x4([0xFE07_0707; 4]);
    assert_eq!(run(VxOp::Vpkpx, false, dropped, dropped), (0, false));
    let a = pack_u32x4([0x0100_0000, 0x0080_0000, 0x0000_0800, 0x0000_0008]);
    let b = pack_u32x4([0x0008_0000, 0x0000_8000, 0x0000_0080, 0]);
    let want = u128::from_be_bytes([
        0x80, 0x00, 0x40, 0x00, 0x00, 0x20, 0x00, 0x01, 0x04, 0x00, 0x02, 0x00, 0x00, 0x10, 0x00,
        0x00,
    ]);
    assert_eq!(run(VxOp::Vpkpx, false, a, b), (want, false));
}

#[test]
fn vd_va_and_vb_may_name_one_register() {
    let v = pack_u32x4([0x0123_FFFF, 0x8000_7F00, 0x0000_0100, 0xFFFF_8000]);
    for op in OPS {
        let mut s = PpuState::new();
        s.set_vr(1, v);
        let insn = PpuInstruction::Vx {
            op,
            rc: false,
            vt: 1,
            va: 1,
            vb: 1,
        };
        assert_eq!(
            exec_no_mem(&insn, &mut s),
            ExecuteVerdict::Continue,
            "{op:?}"
        );
        let got = (s.vr[1], s.vscr() & VSCR_SAT != 0);
        assert_eq!(got, run(op, false, v, v), "{op:?}");
        assert_eq!(got, expected(op, v, v), "{op:?}");
    }
}
