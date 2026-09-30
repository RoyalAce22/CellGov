//! fma, fms and fnms against an exact oracle: the product is exact and
//! unbounded, and only the sum truncates, saturates or flushes.

use super::single_float_tests::{decode_operand, fpscr_of, truncate, SpFlags, CLASSES};
use super::*;
use crate::state::SpuState;
use cellgov_ps3_abi::hw::spu_fpscr::fpscr_field;

/// [SPU-ISA p:28 s:2.3] RRR: 4-bit opcode, RT, RB, RA, RC.
fn rrr(op: u32, rt: u32, rb: u32, ra: u32, rc: u32) -> u32 {
    op << 28 | rt << 21 | rb << 14 | ra << 7 | rc
}

const FMA: u32 = 0xE;
const FMS: u32 = 0xF;
const FNMS: u32 = 0xD;

/// Bits in `m`.
fn width(m: i128) -> i32 {
    128 - m.leading_zeros() as i32
}

/// Truncates the exact sum of two signed integer-scaled values.
fn exact_sum(x: (bool, i128, i32), y: (bool, i128, i32)) -> (u32, SpFlags) {
    match (x.1, y.1) {
        (0, 0) => return (0, SpFlags::default()),
        (0, _) => return truncate(y.0, y.1 as u128, y.2, false),
        (_, 0) => return truncate(x.0, x.1 as u128, x.2, false),
        _ => {}
    }
    let (big, small) = if x.2 + width(x.1) >= y.2 + width(y.1) {
        (x, y)
    } else {
        (y, x)
    };
    let (sn, mut sm, mut se) = small;
    let mut sticky = false;
    // A smaller value whose top bit is more than 30 below the larger one's
    // lowest bit only nudges the sum strictly off it; the kept 24 bits
    // reach at most one bit below that lowest bit, so it stands in as one
    // bit 30 below, with sticky set.
    if se + width(sm) < big.2 - 30 {
        sm = 1;
        se = big.2 - 30;
        sticky = true;
    }
    let e = big.2.min(se);
    let total = |negative: bool, value: i128| if negative { -value } else { value };
    let sum = total(big.0, big.1 << (big.2 - e)) + total(sn, sm << (se - e));
    let (magnitude, sticky) = if sticky && big.0 != sn {
        (sum.unsigned_abs() - 1, true)
    } else {
        (sum.unsigned_abs(), sticky)
    };
    truncate(sum < 0, magnitude, e, sticky)
}

/// The oracle's result for one slot of `op` on `a`, `b` and `c`.
///
/// [SPU-ISA p:208 s:9] fma is RA x RB + RC; [SPU-ISA p:212 s:9] fms is RA x RB - RC; [SPU-ISA p:210 s:9] fnms is RC - RA x RB.
fn oracle(op: u32, a: u32, b: u32, c: u32) -> (u32, SpFlags) {
    let (an, am, ae, ad) = decode_operand(a);
    let (bn, bm, be, bd) = decode_operand(b);
    let (cn, cm, ce, cd) = decode_operand(c);
    let product = ((an != bn) != (op == FNMS), am * bm, ae + be);
    let addend = (cn != (op == FMS), cm, ce);
    let (bits, flags) = exact_sum(product, addend);
    (
        bits,
        SpFlags {
            diff: flags.diff || ad || bd || cd,
            ..flags
        },
    )
}

/// Runs `op` over four slots and returns RT's words and the FPSCR.
fn run(op: u32, a: [u32; 4], b: [u32; 4], c: [u32; 4]) -> ([u32; 4], u128) {
    let mut s = SpuState::new();
    s.regs[1] = from_words(a);
    s.regs[2] = from_words(b);
    s.regs[4] = from_words(c);
    s.regs[3] = [0xAA; 16];
    let insn = crate::decode::decode(rrr(op, 3, 2, 1, 4)).expect("decodes");
    assert_eq!(
        execute(&insn, &mut s, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
    (words(s.regs[3]), s.fpscr)
}

/// Checks all four slots of `op` against the oracle.
fn check(op: u32, a: [u32; 4], b: [u32; 4], c: [u32; 4]) {
    let expected: [(u32, SpFlags); 4] = std::array::from_fn(|i| oracle(op, a[i], b[i], c[i]));
    let (words, fpscr) = run(op, a, b, c);
    let context = format!("op {op:#x} {a:08x?} {b:08x?} {c:08x?}");
    assert_eq!(words, expected.map(|(bits, _)| bits), "{context}");
    assert_eq!(
        fpscr,
        fpscr_of(expected.map(|(_, flags)| flags)),
        "{context}"
    );
}

#[test]
fn every_operand_class_triple_matches_the_oracle() {
    let signed: Vec<u32> = CLASSES
        .iter()
        .flat_map(|&bits| [bits, bits | 0x8000_0000])
        .collect();
    for op in [FMA, FMS, FNMS] {
        for chunk_a in signed.chunks(4) {
            let a: [u32; 4] = chunk_a.try_into().expect("four operands");
            for &b in &signed {
                for &c in &signed {
                    check(op, a, [b; 4], [c; 4]);
                }
            }
        }
    }
}

#[test]
fn every_exponent_gap_between_product_and_addend_matches_the_oracle() {
    // A product with a full 48-bit significand near 1.0, against an addend
    // with a set lowest bit in every binade and every sign combination.
    let a = [0x3FFF_FFFF, 0x3FFF_FFFF, 0xBFFF_FFFF, 0x3F80_0001];
    let b = [0x3FFF_FFFF, 0x3FFF_FFFF, 0x3FFF_FFFF, 0x3F80_0001];
    for op in [FMA, FMS, FNMS] {
        for exponent in 0..=255u32 {
            let c = exponent << 23 | 1;
            check(op, a, b, [c, c | 0x8000_0000, c, c | 0x8000_0000]);
            // A power of two cancels the product's top bits from above.
            let c = exponent << 23;
            check(op, a, b, [c, c | 0x8000_0000, c, c | 0x8000_0000]);
        }
    }
    // The product moves through every binade against a fixed addend.
    for op in [FMA, FMS, FNMS] {
        for exponent in 1..=255u32 {
            let a = [exponent << 23 | 0x7F_FFFF; 4];
            check(
                op,
                a,
                [0x3FFF_FFFF; 4],
                [0x3F80_0001, 0xBF80_0001, 0x0080_0001, 0xFF7F_FFFF],
            );
        }
    }
}

/// [SPU-ISA p:208 s:9] the multiplication is exact and not subject to limits on its range.
#[test]
fn a_product_out_of_range_is_not_saturated_or_flushed_before_the_add() {
    // Slot 0: Smax x 2 is above Smax, and less Smax it is Smax again (DIFF,
    // no OVF). Slot 1: 2^-64 x 2^-64 is below Smin, and plus Smin it is
    // 1.25 x Smin (no UNF). Slots 2 and 3: the same, through fms.
    let (words, fpscr) = run(
        FMA,
        [0x7FFF_FFFF, 0x1F80_0000, 0x7FFF_FFFF, 0x1F80_0000],
        [0x4000_0000, 0x1F80_0000, 0x4000_0000, 0x1F80_0000],
        [0xFFFF_FFFF, 0x0080_0000, 0xFFFF_FFFF, 0x0080_0000],
    );
    assert_eq!(words, [0x7FFF_FFFF, 0x00A0_0000, 0x7FFF_FFFF, 0x00A0_0000]);
    assert_eq!(fpscr, fpscr_field(31, 1) | fpscr_field(95, 1));
    let (words, fpscr) = run(
        FMS,
        [0x7FFF_FFFF, 0x1F80_0000, 0x7FFF_FFFF, 0x1F80_0000],
        [0x4000_0000, 0x1F80_0000, 0x4000_0000, 0x1F80_0000],
        [0x7FFF_FFFF, 0x8080_0000, 0x7FFF_FFFF, 0x8080_0000],
    );
    assert_eq!(words, [0x7FFF_FFFF, 0x00A0_0000, 0x7FFF_FFFF, 0x00A0_0000]);
    assert_eq!(fpscr, fpscr_field(31, 1) | fpscr_field(95, 1));
}

/// [SPU-ISA p:195 s:9.1] every zero result is +0.
#[test]
fn a_product_that_cancels_the_addend_gives_positive_zero() {
    // 3 x 2 against 6 in each sign arrangement that cancels.
    for (op, a, c) in [
        (FMA, 0x4040_0000, 0xC0C0_0000),
        (FMA, 0xC040_0000, 0x40C0_0000),
        (FMS, 0x4040_0000, 0x40C0_0000),
        (FMS, 0xC040_0000, 0xC0C0_0000),
        (FNMS, 0x4040_0000, 0x40C0_0000),
        (FNMS, 0xC040_0000, 0xC0C0_0000),
    ] {
        let (words, fpscr) = run(op, [a; 4], [0x4000_0000; 4], [c; 4]);
        assert_eq!((words, fpscr), ([0; 4], 0), "op {op:#x} {a:08x} {c:08x}");
    }
    // Both the product and the addend zero, in every sign combination.
    for op in [FMA, FMS, FNMS] {
        let (words, _) = run(
            op,
            [0x8000_0000, 0x8000_0000, 0x0000_0000, 0x8000_0000],
            [0x3F80_0000; 4],
            [0x8000_0000, 0x0000_0000, 0x8000_0000, 0x8000_0000],
        );
        assert_eq!(words, [0; 4], "op {op:#x}");
    }
}

#[test]
fn the_product_keeps_its_low_bits_through_the_add() {
    // (1 + 2^-23)^2 = 1 + 2^-22 + 2^-46; less 1 + 2^-22 leaves 2^-46,
    // which a product truncated to 24 bits first would lose.
    let (words, fpscr) = run(FMA, [0x3F80_0001; 4], [0x3F80_0001; 4], [0xBF80_0002; 4]);
    assert_eq!(words, [0x2880_0000; 4]);
    assert_eq!(fpscr, 0);
    let (words, _) = run(FNMS, [0x3F80_0001; 4], [0x3F80_0001; 4], [0x3F80_0002; 4]);
    assert_eq!(words, [0xA880_0000; 4]);
}
