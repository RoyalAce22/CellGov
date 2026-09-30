//! fa, fs and fm against a truncation oracle written from the ISA's
//! single-precision rules, never from a host round-to-nearest result.

use super::*;
use crate::state::SpuState;
use cellgov_ps3_abi::hw::spu_fpscr::fpscr_field;

// [SPU-ISA p:28 s:2.3] RR: 11-bit opcode, RB, RA, RT.
fn rr(op: u32, rt: u32, ra: u32, rb: u32) -> u32 {
    op << 21 | rb << 14 | ra << 7 | rt
}

const FA: u32 = 0x2C4;
const FS: u32 = 0x2C5;
const FM: u32 = 0x2C6;

/// The oracle's flags for one slot: OVF, UNF, DIFF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct SpFlags {
    pub(super) overflow: bool,
    pub(super) underflow: bool,
    pub(super) diff: bool,
}

/// A decoded operand: sign, 24-bit significand (0 for zero), exponent of
/// its lowest bit, and whether reading it raised DIFF.
// [SPU-ISA p:196 s:9.1] a zero exponent reads as zero and raises DIFF with a nonzero fraction; exponent 255 is a number and raises DIFF.
pub(super) fn decode_operand(bits: u32) -> (bool, i128, i32, bool) {
    let negative = bits >> 31 == 1;
    let exponent = (bits >> 23 & 0xFF) as i32;
    let fraction = i128::from(bits & 0x7F_FFFF);
    if exponent == 0 {
        (negative, 0, 0, fraction != 0)
    } else {
        (
            negative,
            fraction | 1 << 23,
            exponent - 127 - 23,
            exponent == 255,
        )
    }
}

/// Truncates `(-1)^negative * magnitude * 2^exponent`, where `sticky`
/// marks nonzero bits below `magnitude`, to extended-range single precision.
// [SPU-ISA p:196 s:9.1] truncation only; below Smin the result is +0 with UNF and DIFF; above Smax it is Smax with OVF; exponent 255 raises DIFF.
pub(super) fn truncate(
    negative: bool,
    magnitude: u128,
    exponent: i32,
    sticky: bool,
) -> (u32, SpFlags) {
    if magnitude == 0 {
        return (0, SpFlags::default());
    }
    let top = 127 - magnitude.leading_zeros() as i32;
    let biased = exponent + top + 127;
    let sign = u32::from(negative) << 31;
    if biased > 255 {
        return (
            sign | 0x7FFF_FFFF,
            SpFlags {
                overflow: true,
                diff: true,
                ..SpFlags::default()
            },
        );
    }
    if biased < 1 {
        return (
            0,
            SpFlags {
                underflow: true,
                diff: true,
                ..SpFlags::default()
            },
        );
    }
    let (kept, dropped) = if top >= 23 {
        let shift = (top - 23) as u32;
        (magnitude >> shift, magnitude & ((1 << shift) - 1) != 0)
    } else {
        (magnitude << (23 - top) as u32, false)
    };
    let bits = sign | (biased as u32) << 23 | (kept as u32 & 0x7F_FFFF);
    (
        bits,
        SpFlags {
            overflow: biased == 255 && kept == 0xFF_FFFF && (dropped || sticky),
            diff: biased == 255,
            ..SpFlags::default()
        },
    )
}

/// The oracle's result for one slot of `op` on `a` and `b`.
fn oracle(op: u32, a: u32, b: u32) -> (u32, SpFlags) {
    let (an, am, ae, ad) = decode_operand(a);
    let (bn, bm, be, bd) = decode_operand(b);
    let bn = if op == FS { !bn } else { bn };
    let (bits, flags) = if op == FM {
        truncate(an != bn, (am * bm) as u128, ae + be, false)
    } else if am == 0 && bm == 0 {
        (0, SpFlags::default())
    } else if am == 0 {
        truncate(bn, bm as u128, be, false)
    } else if bm == 0 {
        truncate(an, am as u128, ae, false)
    } else {
        // Align on the lower exponent. A smaller operand more than 60 bits
        // below the larger one's lowest bit only nudges the value strictly
        // off it, so it stands in as one bit 60 below, with sticky set.
        let (big, small) = if ae + 24 >= be + 24 {
            ((an, am, ae), (bn, bm, be))
        } else {
            ((bn, bm, be), (an, am, ae))
        };
        let (sn, mut sm, mut se) = small;
        let mut sticky = false;
        if big.2 - (se + 24) > 60 {
            sm = 1;
            se = big.2 - 60;
            sticky = true;
        }
        let e = big.2.min(se);
        let x = big.1 << (big.2 - e);
        let y = sm << (se - e);
        let signed = |negative: bool, value: i128| if negative { -value } else { value };
        let total = signed(big.0, x) + signed(sn, y);
        // A sticky nudge subtracted from the larger operand leaves it
        // strictly below the aligned value: one unit less, sticky.
        let (magnitude, sticky) = if sticky && big.0 != sn {
            (total.unsigned_abs() - 1, true)
        } else {
            (total.unsigned_abs(), sticky)
        };
        truncate(total < 0, magnitude, e, sticky)
    };
    (
        bits,
        SpFlags {
            diff: flags.diff || ad || bd,
            ..flags
        },
    )
}

/// Runs `op` over the four slot pairs and returns RT's words and the FPSCR.
fn run(op: u32, a: [u32; 4], b: [u32; 4]) -> ([u32; 4], u128) {
    let mut s = SpuState::new();
    s.regs[1] = from_words(a);
    s.regs[2] = from_words(b);
    s.regs[3] = [0xAA; 16];
    let insn = crate::decode::decode(rr(op, 3, 1, 2)).expect("decodes");
    assert_eq!(
        execute(&insn, &mut s, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
    (words(s.regs[3]), s.fpscr)
}

/// The FPSCR bits the oracle's per-slot flags set.
pub(super) fn fpscr_of(flags: [SpFlags; 4]) -> u128 {
    [29, 61, 93, 125]
        .into_iter()
        .zip(flags)
        .fold(0, |fpscr, (first, flags)| {
            [flags.overflow, flags.underflow, flags.diff]
                .into_iter()
                .enumerate()
                .filter(|(_, set)| *set)
                .fold(fpscr, |fpscr, (offset, _)| {
                    fpscr | fpscr_field(first + offset as u32, 1)
                })
        })
}

/// Checks all four slots of `op` against the oracle.
fn check(op: u32, a: [u32; 4], b: [u32; 4]) {
    let expected: [(u32, SpFlags); 4] = std::array::from_fn(|i| oracle(op, a[i], b[i]));
    let (words, fpscr) = run(op, a, b);
    assert_eq!(
        words,
        expected.map(|(bits, _)| bits),
        "op {op:#05x} {a:08x?} {b:08x?}"
    );
    assert_eq!(
        fpscr,
        fpscr_of(expected.map(|(_, flags)| flags)),
        "op {op:#05x} {a:08x?} {b:08x?}"
    );
}

/// The operand classes: zeros, denormals, Smin and its neighbour, ordinary
/// values, exponent 254, exponent 255 and Smax, with both signs.
pub(super) const CLASSES: [u32; 12] = [
    0x0000_0000,
    0x0000_0001,
    0x007F_FFFF,
    0x0080_0000,
    0x0080_0001,
    0x3F80_0000,
    0x3FC0_0000,
    0x4049_0FDB,
    0x7F00_0000,
    0x7F7F_FFFF,
    0x7F80_0000,
    0x7FFF_FFFF,
];

#[test]
fn every_operand_class_pair_matches_the_oracle() {
    let signed: Vec<u32> = CLASSES
        .iter()
        .flat_map(|&bits| [bits, bits | 0x8000_0000])
        .collect();
    for op in [FA, FS, FM] {
        for chunk_a in signed.chunks(4) {
            for &b in &signed {
                let a: [u32; 4] = chunk_a.try_into().expect("four operands");
                check(op, a, [b; 4]);
            }
        }
    }
}

#[test]
fn every_exponent_gap_matches_the_oracle() {
    // From Smin the gaps reach exponent 255, past where `add` drops bits as sticky.
    for (base, max_gap) in [(0x0080_0000u32, 254u32), (0x4B00_0000, 105)] {
        for op in [FA, FS] {
            for gap in 0..=max_gap {
                // A value with a set lowest bit and a power of two, gap binades
                // apart, in each sign combination.
                let a = base + 1 + (gap << 23);
                let b = base;
                check(
                    op,
                    [a, a | 0x8000_0000, a, base + (gap << 23)],
                    [b, b, b | 0x8000_0000, base | 0x7F_FFFF],
                );
            }
        }
    }
}

// [SPU-ISA p:195 s:9.1] every zero result is +0.
#[test]
fn every_zero_result_is_positive_zero() {
    let (words, fpscr) = run(
        FA,
        [0x8000_0000, 0x3F80_0000, 0x4000_0000, 0x8000_0000],
        [0x8000_0000, 0xBF80_0000, 0x4000_0000, 0x0000_0000],
    );
    assert_eq!(words, [0, 0, 0x4080_0000, 0]);
    assert_eq!(fpscr, 0);
    let (words, _) = run(FS, [0x3F80_0000; 4], [0x3F80_0000; 4]);
    assert_eq!(words, [0; 4]);
    let (words, _) = run(FM, [0x8000_0000; 4], [0x3F80_0000; 4]);
    assert_eq!(words, [0; 4]);
}

// [SPU-ISA p:196 s:9.1] overflow saturates to Smax with the result's sign and sets OVF and DIFF; underflow gives +0 with UNF and DIFF.
#[test]
fn overflow_saturates_and_underflow_flushes_in_their_own_slot() {
    let (words, fpscr) = run(
        FM,
        [0x3F80_0000, 0x3F80_0000, 0xFF00_0000, 0x0080_0000],
        [0x3F80_0000, 0x3F80_0000, 0x7F00_0000, 0x0080_0000],
    );
    assert_eq!(words, [0x3F80_0000, 0x3F80_0000, 0xFFFF_FFFF, 0]);
    // Slot 2 overflows (OVF, DIFF), slot 3 underflows (UNF, DIFF).
    assert_eq!(
        fpscr,
        fpscr_field(93, 1) | fpscr_field(95, 1) | fpscr_field(126, 2)
    );
}

// [SPU-ISA p:196 s:9.1] a large gap truncates: a power of two less a tiny value is the all-ones significand one binade down.
#[test]
fn a_power_of_two_less_a_tiny_value_truncates_one_binade_down() {
    let (words, _) = run(FS, [0x4000_0000; 4], [0x0080_0000; 4]);
    assert_eq!(words, [0x3FFF_FFFF; 4]);
}

/// Every single-precision value times four fixed second operands, one per
/// slot, against the host `f64` product, which is exact at 48 bits.
#[test]
#[ignore = "exhaustive over 2^32 first operands; run with --release -- --ignored"]
fn fm_over_every_first_operand_matches_the_exact_host_product() {
    let seconds = [0x3F80_0000u32, 0x4049_0FDB, 0x0080_0000, 0xFF7F_FFFF];
    let value = |bits: u32| {
        let (negative, magnitude, exponent, _) = decode_operand(bits);
        let v = magnitude as f64 * 2f64.powi(exponent);
        if negative {
            -v
        } else {
            v
        }
    };
    let insn = crate::decode::decode(rr(FM, 3, 1, 2)).expect("decodes");
    let mut s = SpuState::new();
    s.regs[2] = from_words(seconds);
    for a in 0..=u32::MAX {
        s.regs[1] = from_words([a; 4]);
        s.fpscr = 0;
        execute(&insn, &mut s, UnitId::new(0));
        let expected: [(u32, SpFlags); 4] = std::array::from_fn(|slot| {
            let b = seconds[slot];
            let product = value(a) * value(b);
            let (_, _, _, a_diff) = decode_operand(a);
            let (_, _, _, b_diff) = decode_operand(b);
            // Decompose the exact f64 product back into an integer and a power.
            let (bits, flags) = if product == 0.0 {
                (0, SpFlags::default())
            } else {
                let raw = product.to_bits();
                let exponent = (raw >> 52 & 0x7FF) as i32 - 1023 - 52;
                let magnitude = u128::from(raw & ((1 << 52) - 1) | 1 << 52);
                truncate(product < 0.0, magnitude, exponent, false)
            };
            (
                bits,
                SpFlags {
                    diff: flags.diff || a_diff || b_diff,
                    ..flags
                },
            )
        });
        assert_eq!(
            words(s.regs[3]),
            expected.map(|(bits, _)| bits),
            "{a:#010x}"
        );
        assert_eq!(
            s.fpscr,
            fpscr_of(expected.map(|(_, flags)| flags)),
            "{a:#010x}"
        );
    }
}
