//! frest, frsqest and fi: fi against its formula, and both estimates
//! through the documented Newton-Raphson sequences against their bounds.

use super::single_float_tests::{fpscr_of, truncate, SpFlags};
use super::*;
use crate::state::SpuState;
use cellgov_ps3_abi::hw::spu_fpscr::fpscr_field;

// [SPU-ISA p:28 s:2.3] RR: 11-bit opcode, RB, RA, RT; RRR: 4-bit opcode, RT, RB, RA, RC.
fn rr(op: u32, rt: u32, ra: u32, rb: u32) -> u32 {
    op << 21 | rb << 14 | ra << 7 | rt
}
fn rrr(op: u32, rt: u32, rb: u32, ra: u32, rc: u32) -> u32 {
    op << 28 | rt << 21 | rb << 14 | ra << 7 | rc
}

const FREST: u32 = 0x1B8;
const FRSQEST: u32 = 0x1B9;
const FI: u32 = 0x3D4;
const FM: u32 = 0x2C6;
const AND: u32 = 0x0C1;
const FMA: u32 = 0xE;
const FNMS: u32 = 0xD;

const ONE: u32 = 0x3F80_0000;
const HALF: u32 = 0x3F00_0000;

fn step(s: &mut SpuState, raw: u32) {
    let insn = crate::decode::decode(raw).expect("decodes");
    assert_eq!(execute(&insn, s, UnitId::new(0)), SpuStepOutcome::Continue);
}

/// The fi oracle for one slot: the formula on RB's fields and RA's
/// fraction, exact, then truncated.
fn fi_oracle(ra: u32, rb: u32) -> (u32, SpFlags) {
    let y = i128::from(ra & 0x7_FFFF);
    let exponent = (rb >> 23 & 0xFF) as i32;
    let base = i128::from(rb >> 10 & 0x1FFF);
    let step = i128::from(rb & 0x3FF);
    // In units of 2^-32: 1.BaseFraction is (2^13 + base) x 2^19, and
    // 0.000StepFraction x Y is step x y.
    let value = ((1 << 13) + base) * (1 << 19) - step * y;
    let (bits, flags) = truncate(rb >> 31 == 1, value as u128, exponent - 127 - 32, false);
    (
        bits,
        SpFlags {
            diff: flags.diff || exponent == 255,
            ..flags
        },
    )
}

fn check_fi(ra: [u32; 4], rb: [u32; 4]) {
    let mut s = SpuState::new();
    s.regs[1] = from_words(ra);
    s.regs[2] = from_words(rb);
    step(&mut s, rr(FI, 3, 1, 2));
    let expected: [(u32, SpFlags); 4] = std::array::from_fn(|i| fi_oracle(ra[i], rb[i]));
    let context = format!("{ra:08x?} {rb:08x?}");
    assert_eq!(
        words(s.regs[3]),
        expected.map(|(bits, _)| bits),
        "{context}"
    );
    assert_eq!(s.fpscr, fpscr_of(expected.map(|(_, f)| f)), "{context}");
}

#[test]
fn fi_matches_its_formula() {
    // Directed, Y all ones: step 0, base 0 with the largest step (the value
    // drops below 1 and renormalizes), both fields all ones, exponent 0;
    // then exponents 129 (negative), 255 and 1.
    check_fi(
        [0x0007_FFFF, 0x0007_FFFF, 0x0007_FFFF, 0x0007_FFFF],
        [0x3F80_0000, 0x3F80_03FF, 0x3FFF_FFFF, 0x0000_03FF],
    );
    check_fi(
        [0x0000_0000, 0x1234_5678, 0x0004_0000, 0xFFFF_FFFF],
        [0xC0FF_E155, 0x7FFF_FC00, 0x7F80_03FF, 0x00FF_FFFF],
    );
    // A spread of pseudo-random pairs.
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state as u32
    };
    for _ in 0..4096 {
        let ra = [next(), next(), next(), next()];
        let rb = [next(), next(), next(), next()];
        check_fi(ra, rb);
    }
}

/// Runs the reciprocal sequence on four operands and returns y2.
// [SPU-ISA p:215 s:9] FREST y0,x; FI y1,x,y0; FNMS t1,x,y1,ONE; FMA y2,t1,y1,y1.
fn reciprocal(x: [u32; 4]) -> ([u32; 4], u128) {
    let mut s = SpuState::new();
    s.regs[1] = from_words(x);
    s.regs[6] = from_words([ONE; 4]);
    step(&mut s, rr(FREST, 2, 1, 0));
    step(&mut s, rr(FI, 3, 1, 2));
    step(&mut s, rrr(FNMS, 4, 3, 1, 6));
    step(&mut s, rrr(FMA, 5, 3, 4, 3));
    (words(s.regs[5]), s.fpscr)
}

/// Runs the reciprocal-square-root sequence on four operands and returns y2.
// [SPU-ISA p:217 s:9] FRSQEST y0,x; AND ax,x,mask; FI y1,ax,y0; FM t1,ax,y1; FM t2,y1,HALF; FNMS t1,t1,y1,ONE; FMA y2,t1,t2,y1.
fn reciprocal_sqrt(x: [u32; 4]) -> [u32; 4] {
    let mut s = SpuState::new();
    s.regs[1] = from_words(x);
    s.regs[6] = from_words([ONE; 4]);
    s.regs[7] = from_words([HALF; 4]);
    s.regs[8] = from_words([0x7FFF_FFFF; 4]);
    step(&mut s, rr(FRSQEST, 2, 1, 0));
    step(&mut s, rr(AND, 9, 1, 8));
    step(&mut s, rr(FI, 3, 9, 2));
    step(&mut s, rr(FM, 4, 9, 3));
    step(&mut s, rr(FM, 10, 3, 7));
    step(&mut s, rrr(FNMS, 4, 3, 4, 6));
    step(&mut s, rrr(FMA, 5, 10, 4, 3));
    words(s.regs[5])
}

/// The largest single-precision Y with x * Y < 1, for a normal x whose
/// reciprocal is normal.
fn reciprocal_floor(x: u32) -> u32 {
    let e = x >> 23 & 0xFF;
    let mx = u64::from(x & 0x7F_FFFF | 1 << 23);
    // x = Mx x 2^ex, Y = My x 2^(-ex - 47): x * Y < 1 is Mx * My < 2^47.
    let my = ((1u64 << 47) - 1) / mx;
    x & 0x8000_0000 | (253 - e) << 23 | (my as u32 & 0x7F_FFFF)
}

/// The largest single-precision Y with x * Y^2 < 1, for a positive normal x.
fn reciprocal_sqrt_floor(x: u32) -> u32 {
    let e = (x >> 23 & 0xFF) as i32;
    let mx = u128::from(x & 0x7F_FFFF | 1 << 23);
    let ex = e - 127 - 23;
    let isqrt = |n: u128| {
        let mut r = (n as f64).sqrt() as u128;
        while r * r > n {
            r -= 1;
        }
        while (r + 1) * (r + 1) <= n {
            r += 1;
        }
        r
    };
    // Y = My x 2^ey: x * Y^2 < 1 is Mx * My^2 < 2^(-ex - 2 ey).
    for ey in (-200..=100).rev() {
        let shift = -ex - 2 * ey;
        if !(0..=120).contains(&shift) {
            continue;
        }
        let my = isqrt(((1u128 << shift) - 1) / mx);
        if my >> 23 == 1 {
            return ((ey + 23 + 127) as u32) << 23 | (my as u32 & 0x7F_FFFF);
        }
    }
    unreachable!("no binade holds the reciprocal square root of {x:#010x}")
}

/// Checks the reciprocal sequence for every fraction in `fractions` at
/// biased exponent `e`, four at a time.
fn check_reciprocal(e: u32, fractions: impl Iterator<Item = u32>) {
    let fractions: Vec<u32> = fractions.collect();
    for chunk in fractions.chunks(4) {
        let mut x = [e << 23; 4];
        for (slot, fraction) in chunk.iter().enumerate() {
            x[slot] |= fraction;
        }
        let (y2, _) = reciprocal(x);
        for slot in 0..chunk.len() {
            // [SPU-ISA p:216 s:9] either y2 = Y or INC(y2) = Y.
            let want = reciprocal_floor(x[slot]);
            assert!(
                y2[slot] == want || y2[slot] + 1 == want,
                "1/{:#010x}: y2 {:#010x}, Y {want:#010x}",
                x[slot],
                y2[slot]
            );
        }
    }
}

/// Checks the reciprocal-square-root sequence likewise.
fn check_reciprocal_sqrt(e: u32, fractions: impl Iterator<Item = u32>) {
    let fractions: Vec<u32> = fractions.collect();
    for chunk in fractions.chunks(4) {
        let mut x = [e << 23; 4];
        for (slot, fraction) in chunk.iter().enumerate() {
            x[slot] |= fraction;
        }
        let y2 = reciprocal_sqrt(x);
        for slot in 0..chunk.len() {
            // [SPU-ISA p:218 s:9] |Y - y2| <= 1 ulp.
            let want = reciprocal_sqrt_floor(x[slot]);
            assert!(
                y2[slot].abs_diff(want) <= 1,
                "1/sqrt({:#010x}): y2 {:#010x}, Y {want:#010x}",
                x[slot],
                y2[slot]
            );
        }
    }
}

/// Every 509th fraction plus both ends of every table segment.
fn sampled() -> impl Iterator<Item = u32> {
    let ends = (0..32u32).flat_map(|i| [i << 18, (i << 18) | 0x3_FFFF]);
    (0..1 << 23).step_by(509).chain(ends)
}

#[test]
fn the_reciprocal_sequence_meets_its_bound() {
    for e in [1, 64, 127, 128, 200, 252] {
        check_reciprocal(e, sampled());
        // The same magnitudes, negative.
        check_reciprocal(e | 0x100, sampled());
    }
}

#[test]
fn the_reciprocal_sqrt_sequence_meets_its_bound() {
    for e in [1, 2, 64, 127, 128, 200, 254, 255] {
        check_reciprocal_sqrt(e, sampled());
    }
}

#[test]
#[ignore = "exhaustive over 2^23 fractions per exponent parity; run with --release -- --ignored"]
fn both_sequences_meet_their_bounds_for_every_fraction() {
    check_reciprocal(127, 0..1 << 23);
    check_reciprocal(128, 0..1 << 23);
    check_reciprocal_sqrt(127, 0..1 << 23);
    check_reciprocal_sqrt(128, 0..1 << 23);
}

// [SPU-ISA p:215 s:9] 1/0 gives 0x7FFFFFFF; [SPU-ISA p:216 s:9] |x| >= 2^126 gives 0, 0x7E800000 included.
#[test]
fn zero_and_big_operands_meet_their_documented_results() {
    let (y2, fpscr) = reciprocal([0, 0x7E80_0000, 0x7F7F_FFFF, 0x7E7F_FFFF]);
    assert_eq!(y2[..3], [0x7FFF_FFFF, 0, 0]);
    assert_ne!(y2[3], 0, "2^126 less one ulp has a normal reciprocal");
    assert_ne!(fpscr & fpscr_field(116, 1), 0, "DBZ in slot 0");
    for e in 253..=255u32 {
        let (y2, _) = reciprocal([
            e << 23,
            e << 23 | 0x7F_FFFF,
            e << 23 | 1,
            e << 23 | 0x40_0000,
        ]);
        assert_eq!(y2, [0; 4], "exponent {e}");
    }
}

// [SPU-ISA p:217 s:9] with a zero exponent, fraction <= 0x000ff53c gives y2 = 0x7fffffff, and above it y2 >= 0x7fc00000.
fn check_zero_threshold(fractions: impl Iterator<Item = u32>) {
    let fractions: Vec<u32> = fractions.collect();
    for chunk in fractions.chunks(4) {
        let mut x = [0; 4];
        x[..chunk.len()].copy_from_slice(chunk);
        let y2 = reciprocal_sqrt(x);
        for slot in 0..chunk.len() {
            if x[slot] <= 0x000F_F53C {
                assert_eq!(y2[slot], 0x7FFF_FFFF, "{:#010x}", x[slot]);
            } else {
                assert!(
                    (0x7FC0_0000..0x7FFF_FFFF).contains(&y2[slot]),
                    "{:#010x}: {:#010x}",
                    x[slot],
                    y2[slot]
                );
            }
        }
    }
}

#[test]
fn the_frsqest_zero_threshold_falls_at_0x000ff53c() {
    check_zero_threshold((0x000F_F530..0x000F_F550).chain(sampled()));
}

#[test]
#[ignore = "exhaustive over 2^23 zero-exponent fractions; run with --release -- --ignored"]
fn the_frsqest_zero_threshold_holds_for_every_fraction() {
    check_zero_threshold(0..1 << 23);
}

// [SPU-ISA p:215 s:9] and [SPU-ISA p:217 s:9]: a zero exponent flags divide by zero, in its own slot.
#[test]
fn a_zero_exponent_flags_divide_by_zero_in_its_own_slot() {
    for op in [FREST, FRSQEST] {
        let mut s = SpuState::new();
        s.regs[1] = from_words([0x0000_0000, ONE, 0x8000_0001, 0x7F80_0000]);
        step(&mut s, rr(op, 2, 1, 0));
        assert_eq!(
            s.fpscr,
            fpscr_field(116, 1) | fpscr_field(118, 1),
            "op {op:#x}"
        );
    }
}

// [SPU-ISA p:215 s:9] S is the operand's sign; [SPU-ISA p:217 s:9] frsqest's sign is always 0.
#[test]
fn the_estimate_carries_the_documented_sign_and_layout() {
    let mut s = SpuState::new();
    s.regs[1] = from_words([ONE, ONE | 0x8000_0000, 0x4000_0000, 0xC000_0000]);
    step(&mut s, rr(FREST, 2, 1, 0));
    step(&mut s, rr(FRSQEST, 3, 1, 0));
    let frest = words(s.regs[2]);
    let frsqest = words(s.regs[3]);
    // 1/1 sits in the binade below 2 x 2^-1; 1/2 one binade lower.
    assert_eq!(frest.map(|w| w >> 23), [126, 0x100 | 126, 125, 0x100 | 125]);
    assert!(frsqest.iter().all(|w| w >> 31 == 0));
    // 1/sqrt(1) and 1/sqrt(2) share a binade; the tables differ by parity.
    assert_eq!(frsqest.map(|w| w >> 23), [126, 126, 126, 126]);
    assert_ne!(frsqest[0], frsqest[2]);
}
