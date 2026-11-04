//! The nine integer multiply-add and multiply-sum instructions and the
//! SAT they set, against a wide-integer reference written from the PEM
//! pseudocode.

use super::lane_support::{lane, pack_u32x4, with_lane};
use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::VaOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::VSCR_SAT;

/// Runs `op` as vD, vA, vB, vC from a state whose SAT is `sat`; returns vD
/// and SAT after.
fn run(op: VaOp, sat: bool, a: u128, b: u128, c: u128) -> (u128, bool) {
    let mut s = PpuState::new();
    if sat {
        s.set_sat();
    }
    s.set_vr(1, a);
    s.set_vr(2, b);
    s.set_vr(4, c);
    let insn = PpuInstruction::Va {
        op,
        vt: 3,
        va: 1,
        vb: 2,
        vc: 4,
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

/// `v` saturated to `w` bits, and whether it saturated.
fn sat(v: i64, w: u32, signed: bool) -> (u64, bool) {
    let (min, max) = if signed {
        (-(1i64 << (w - 1)), (1i64 << (w - 1)) - 1)
    } else {
        (0, (1i64 << w) - 1)
    };
    let r = v.clamp(min, max);
    ((r as u64) & ((1u64 << w) - 1), r != v)
}

/// One half-word element of `vmhaddshs` (`round` 0) or `vmhraddshs`
/// (`round` 0x4000).
///
/// [AltiVec-PEM p:6-79 s:6.2] vmhaddshs: prod0:31 = vA x vB signed; temp0:16 = prod0:16 + SignExtend(vC, 17); vD = SItoSIsat(temp, 16).
/// [AltiVec-PEM p:6-80 s:6.2] vmhraddshs: prod0:31 = prod0:31 + 0x0000_4000, then bits 0:16 of that sum go on.
fn mhadd_reference(a: u64, b: u64, c: u64, round: i64) -> (u64, bool) {
    let prod = (sx(a, 16) * sx(b, 16) + round) as u64 & 0xFFFF_FFFF;
    // Bits 0:16 of the 32-bit product: its high 17 bits, as a signed value.
    let high17 = sx(prod >> 15, 17);
    sat(high17 + sx(c, 16), 16, true)
}

/// One word of a multiply-sum: `c` plus the products of the `sub`-bit
/// sub-elements of `a` and `b`.
///
/// [AltiVec-PEM p:6-95 s:6.2] vmsummbm: the vC word plus the four signed vA byte x unsigned vB byte products.
/// [AltiVec-PEM p:6-97 s:6.2] vmsumshs: the vC word, sign-extended to 34 bits, plus the two signed products, saturated to a signed word.
/// [AltiVec-PEM p:6-100 s:6.2] vmsumuhs: the same, unsigned, saturated to an unsigned word.
fn msum_reference(a: u64, b: u64, c: u64, sub: u32, signed_a: bool, signed_b: bool) -> i64 {
    let mask = (1u64 << sub) - 1;
    let mut total = if signed_a && signed_b {
        sx(c, 32)
    } else {
        c as i64
    };
    for j in 0..32 / sub {
        let shift = 32 - sub * (j + 1);
        let (x, y) = ((a >> shift) & mask, (b >> shift) & mask);
        let x = if signed_a { sx(x, sub) } else { x as i64 };
        let y = if signed_b { sx(y, sub) } else { y as i64 };
        total += x * y;
    }
    total
}

fn classes16() -> [u64; 6] {
    [0, 1, 0xFFFF, 0x7FFF, 0x8000, 0x9E37]
}

fn classes32() -> [u64; 6] {
    [0, 1, 0xFFFF_FFFF, 0x7FFF_FFFF, 0x8000_0000, 0x9E37_79B9]
}

/// A vA, a vB, a vC, and the element triples they hold, element 0 first.
type ClassTripleVector = (u128, u128, u128, Vec<(u64, u64, u64)>);

/// Every triple of `classes`, packed `128 / w` triples per vector.
fn triples(w: u32, classes: &[u64]) -> Vec<ClassTripleVector> {
    let all: Vec<(u64, u64, u64)> = classes
        .iter()
        .flat_map(|&a| {
            classes
                .iter()
                .flat_map(move |&b| classes.iter().map(move |&c| (a, b, c)))
        })
        .collect();
    all.chunks((128 / w) as usize)
        .map(|chunk| {
            let (mut a, mut b, mut c) = (0u128, 0u128, 0u128);
            for (i, &(x, y, z)) in (0..).zip(chunk) {
                a = with_lane(a, w, i, x);
                b = with_lane(b, w, i, y);
                c = with_lane(c, w, i, z);
            }
            (a, b, c, chunk.to_vec())
        })
        .collect()
}

#[test]
fn the_half_word_multiply_adds_match_the_reference_over_every_class_triple() {
    for op in [VaOp::Vmhaddshs, VaOp::Vmhraddshs, VaOp::Vmladduhm] {
        let mut checked = 0;
        for (a, b, c, chunk) in triples(16, &classes16()) {
            let (r, s) = run(op, false, a, b, c);
            let mut any = false;
            for (i, &(x, y, z)) in (0..).zip(&chunk) {
                let (want, clamps) = match op {
                    VaOp::Vmhaddshs => mhadd_reference(x, y, z, 0),
                    VaOp::Vmhraddshs => mhadd_reference(x, y, z, 0x4000),
                    // [AltiVec-PEM p:6-88 s:6.2] vmladduhm: vD = the low half word of vA x vB + vC.
                    _ => ((x * y + z) & 0xFFFF, false),
                };
                assert_eq!(lane(r, 16, i), want, "{op:?} {x:#x} {y:#x} {z:#x}");
                any |= clamps;
                checked += 1;
            }
            assert_eq!(s, any, "{op:?} SAT");
        }
        assert_eq!(checked, 216, "{op:?}");
    }
}

#[test]
fn the_multiply_sums_match_the_reference_over_every_class_triple() {
    // op, sub-element width, vA signed, vB signed, saturating.
    let ops = [
        (VaOp::Vmsumubm, 8, false, false, false),
        (VaOp::Vmsummbm, 8, true, false, false),
        (VaOp::Vmsumuhm, 16, false, false, false),
        (VaOp::Vmsumuhs, 16, false, false, true),
        (VaOp::Vmsumshm, 16, true, true, false),
        (VaOp::Vmsumshs, 16, true, true, true),
    ];
    for (op, sub, signed_a, signed_b, saturating) in ops {
        let mut checked = 0;
        // Each class value is a whole vA, vB or vC word; the vA and vB
        // words split into their byte or half-word sub-elements.
        for (a, b, c, chunk) in triples(32, &classes32()) {
            let (r, s) = run(op, false, a, b, c);
            let mut any = false;
            for (i, &(x, y, z)) in (0..).zip(&chunk) {
                let total = msum_reference(x, y, z, sub, signed_a, signed_b);
                let (want, clamps) = if saturating {
                    sat(total, 32, signed_a)
                } else {
                    ((total as u64) & 0xFFFF_FFFF, false)
                };
                assert_eq!(lane(r, 32, i), want, "{op:?} {x:#x} {y:#x} {z:#x}");
                any |= clamps;
                checked += 1;
            }
            assert_eq!(s, any, "{op:?} SAT");
        }
        assert_eq!(checked, 216, "{op:?}");
    }
}

fn halfs(x: u16) -> u128 {
    (0..8).fold(0u128, |v, _| (v << 16) | u128::from(x))
}

#[test]
fn the_high_multiply_adds_clamp_round_and_shift_arithmetically() {
    // (-32768) x (-32768) >> 15 is 32768: it clamps with vC = 0 and lands
    // on the bound with vC = -1.
    assert_eq!(
        run(VaOp::Vmhaddshs, false, halfs(0x8000), halfs(0x8000), 0),
        (halfs(0x7FFF), true)
    );
    assert_eq!(
        run(
            VaOp::Vmhaddshs,
            false,
            halfs(0x8000),
            halfs(0x8000),
            halfs(0xFFFF)
        ),
        (halfs(0x7FFF), false)
    );
    // 1 x 0x4000 = 0x4000: >> 15 is 0, and with + 0x4000 it is 0x8000 >> 15 = 1.
    assert_eq!(
        run(VaOp::Vmhaddshs, false, halfs(1), halfs(0x4000), 0),
        (0, false)
    );
    assert_eq!(
        run(VaOp::Vmhraddshs, false, halfs(1), halfs(0x4000), 0),
        (halfs(1), false)
    );
    // -1 x 1 = -1, and an arithmetic >> 15 of it is -1.
    assert_eq!(
        run(VaOp::Vmhaddshs, false, halfs(0xFFFF), halfs(1), 0),
        (halfs(0xFFFF), false)
    );
}

#[test]
fn vmladduhm_wraps() {
    let all = halfs(0xFFFF);
    // 0xFFFF x 0xFFFF + 0xFFFF = 0xFFFF_0000; its low half word is 0.
    assert_eq!(run(VaOp::Vmladduhm, false, all, all, all), (0, false));
}

#[test]
fn vmsummbm_reads_va_signed_and_vb_unsigned() {
    // vA byte -1 times vB byte 255 is -255, once per word.
    let a = pack_u32x4([0xFF00_0000; 4]);
    let b = pack_u32x4([0xFF00_0000; 4]);
    assert_eq!(
        run(VaOp::Vmsummbm, false, a, b, 0),
        (pack_u32x4([0xFFFF_FF01; 4]), false)
    );
}

#[test]
fn vmsumuhs_clamps_only_when_a_product_pushes_past_the_top() {
    let top = pack_u32x4([0xFFFF_FFFF; 4]);
    let one = pack_u32x4([0x0001_0000; 4]);
    assert_eq!(run(VaOp::Vmsumuhs, false, one, one, top), (top, true));
    assert_eq!(run(VaOp::Vmsumuhs, false, 0, one, top), (top, false));
}

#[test]
fn vmsumshs_clamps_both_ways_and_sums_in_34_bits_first() {
    let words = |x: u32| pack_u32x4([x; 4]);
    // 0x7FFF_FFFF + 0x7FFF x 0x7FFF clamps up; 0x8000_0000 + 0x7FFF x
    // -0x8000 clamps down.
    assert_eq!(
        run(
            VaOp::Vmsumshs,
            false,
            words(0x7FFF_0000),
            words(0x7FFF_0000),
            words(0x7FFF_FFFF)
        ),
        (words(0x7FFF_FFFF), true)
    );
    assert_eq!(
        run(
            VaOp::Vmsumshs,
            false,
            words(0x7FFF_0000),
            words(0x8000_0000),
            words(0x8000_0000)
        ),
        (words(0x8000_0000), true)
    );
    // The two products, 0x7FFF x 0x7FFF and 0x7FFF x -0x7FFF, cancel; vC +
    // the first alone would clamp.
    assert_eq!(
        run(
            VaOp::Vmsumshs,
            false,
            words(0x7FFF_7FFF),
            words(0x7FFF_8001),
            words(0x7FFF_FFFF)
        ),
        (words(0x7FFF_FFFF), false)
    );
}

/// [AltiVec-PEM p:2-5 s:2.3.2] SAT stays set until mtvscr clears it.
#[test]
fn sat_sticks_and_the_modulo_forms_leave_it_alone() {
    let all = u128::MAX;
    for op in [
        VaOp::Vmhaddshs,
        VaOp::Vmhraddshs,
        VaOp::Vmladduhm,
        VaOp::Vmsumubm,
        VaOp::Vmsummbm,
        VaOp::Vmsumuhm,
        VaOp::Vmsumuhs,
        VaOp::Vmsumshm,
        VaOp::Vmsumshs,
    ] {
        assert!(run(op, true, 0, 0, 0).1, "{op:?} keeps SAT");
    }
    for op in [
        VaOp::Vmladduhm,
        VaOp::Vmsumubm,
        VaOp::Vmsummbm,
        VaOp::Vmsumuhm,
        VaOp::Vmsumshm,
    ] {
        assert!(!run(op, false, all, all, all).1, "{op:?} sets no SAT");
    }
}
