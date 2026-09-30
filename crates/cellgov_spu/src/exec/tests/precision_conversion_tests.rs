//! frds and fesd against an integer IEEE 754 oracle, with the CBE's
//! flushed denormal inputs and default NaN, in all four rounding modes.

use super::*;
use crate::exec::lanes::{doublewords, from_doublewords};
use crate::state::SpuState;
use cellgov_ps3_abi::hw::spu_fpscr::{fpscr_field, FPSCR_DOUBLE_FIRST, FPSCR_RN_FIRST};

// [SPU-ISA p:28 s:2.3] RR: 11-bit opcode, RB, RA, RT.
fn rr(op: u32, rt: u32, ra: u32) -> u32 {
    op << 21 | ra << 7 | rt
}

const FESD: u32 = 0x3B8;
const FRDS: u32 = 0x3B9;

const SINGLE_NAN: u64 = 0x7FC0_0000;
const DOUBLE_NAN: u64 = 0x7FF8_0000_0000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Nearest = 0,
    Zero = 1,
    Up = 2,
    Down = 3,
}
const MODES: [Mode; 4] = [Mode::Nearest, Mode::Zero, Mode::Up, Mode::Down];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct DFlags {
    overflow: bool,
    underflow: bool,
    inexact: bool,
    invalid: bool,
    nan: bool,
    denormal: bool,
}

/// Rounds `(-1)^neg * mag * 2^exp` into a format with `frac` fraction
/// bits and exponent `bias`, in `mode`.
// [SPU-ISA p:199 s:9.2.2] UNF is tininess before rounding together with an inexact result.
fn round_ieee(neg: bool, mag: u128, exp: i32, mode: Mode, frac: i32, bias: i32) -> (u64, DFlags) {
    let sign = u64::from(neg) << (frac + (bias + 1).trailing_zeros() as i32 + 1);
    let top = 127 - mag.leading_zeros() as i32;
    let binade = exp + top;
    let emin = 1 - bias;
    let tiny = binade < emin;
    let lsb = binade.max(emin) - frac;
    let (mut kept, guard, lower) = if exp >= lsb {
        (mag << (exp - lsb), false, false)
    } else {
        let d = (lsb - exp) as u32;
        if d > 127 {
            (0, false, mag != 0)
        } else {
            (
                mag >> d,
                mag >> (d - 1) & 1 == 1,
                mag & ((1 << (d - 1)) - 1) != 0,
            )
        }
    };
    let inexact = guard || lower;
    let up = match mode {
        Mode::Nearest => guard && (lower || kept & 1 == 1),
        Mode::Zero => false,
        Mode::Up => inexact && !neg,
        Mode::Down => inexact && neg,
    };
    kept += u128::from(up);
    let mut biased = binade + bias;
    if !tiny && kept >> (frac + 1) != 0 {
        kept >>= 1;
        biased += 1;
    }
    let flags = DFlags {
        inexact,
        underflow: tiny && inexact,
        ..DFlags::default()
    };
    let exp_max = 2 * bias + 1;
    if !tiny && biased >= exp_max {
        let to_max = match mode {
            Mode::Nearest => false,
            Mode::Zero => true,
            Mode::Up => neg,
            Mode::Down => !neg,
        };
        let magnitude = if to_max {
            ((exp_max as u64 - 1) << frac) | ((1 << frac) - 1)
        } else {
            (exp_max as u64) << frac
        };
        return (
            sign | magnitude,
            DFlags {
                overflow: true,
                inexact: true,
                ..flags
            },
        );
    }
    let field = if tiny {
        kept as u64
    } else {
        (biased as u64) << frac | (kept as u64 & ((1 << frac) - 1))
    };
    (sign | field, flags)
}

/// The frds oracle: a double rounded to a single word.
// [SPU-ISA p:198 s:9.2.1] IEEE 754 but for denormal inputs, which an implementation may read as zero with DENORM; [Mueller2005 p:61 s:3.2] the CBE's double unit, which does the conversions, reads denormal operands as zero.
fn frds_oracle(bits: u64, mode: Mode) -> (u64, DFlags) {
    let neg = bits >> 63 == 1;
    let e = (bits >> 52 & 0x7FF) as i32;
    let f = bits & ((1 << 52) - 1);
    let none = DFlags::default();
    match (e, f) {
        (0x7FF, 0) => (u64::from(neg) << 31 | 0x7F80_0000, none),
        (0x7FF, _) => (
            SINGLE_NAN,
            DFlags {
                nan: true,
                invalid: f >> 51 == 0,
                ..none
            },
        ),
        (0, 0) => (u64::from(neg) << 31, none),
        (0, _) => (
            u64::from(neg) << 31,
            DFlags {
                denormal: true,
                ..none
            },
        ),
        _ => round_ieee(neg, u128::from(f | 1 << 52), e - 1075, mode, 23, 127),
    }
}

/// The fesd oracle: a single word extended to a double, by its fields.
fn fesd_oracle(word: u32) -> (u64, DFlags) {
    let sign = u64::from(word >> 31) << 63;
    let e = u64::from(word >> 23 & 0xFF);
    let f = u64::from(word & 0x7F_FFFF);
    let none = DFlags::default();
    match (e, f) {
        (0xFF, 0) => (sign | 0x7FF0_0000_0000_0000, none),
        (0xFF, _) => (
            DOUBLE_NAN,
            DFlags {
                nan: true,
                invalid: f >> 22 == 0,
                ..none
            },
        ),
        (0, 0) => (sign, none),
        (0, _) => (
            sign,
            DFlags {
                denormal: true,
                ..none
            },
        ),
        _ => (sign | (e + 1023 - 127) << 52 | f << 29, none),
    }
}

fn fpscr_of(flags: [DFlags; 2]) -> u128 {
    FPSCR_DOUBLE_FIRST
        .into_iter()
        .zip(flags)
        .fold(0, |fpscr, (first, f)| {
            [
                f.overflow,
                f.underflow,
                f.inexact,
                f.invalid,
                f.nan,
                f.denormal,
            ]
            .into_iter()
            .enumerate()
            .filter(|(_, set)| *set)
            .fold(fpscr, |fpscr, (offset, _)| {
                fpscr | fpscr_field(first + offset as u32, 1)
            })
        })
}

fn rounding_fields(modes: [Mode; 2]) -> u128 {
    FPSCR_RN_FIRST
        .into_iter()
        .zip(modes)
        .fold(0, |fpscr, (first, mode)| {
            fpscr | (mode as u128) << (128 - first - 2)
        })
}

fn run(op: u32, a: [u64; 2], modes: [Mode; 2]) -> ([u64; 2], u128) {
    let mut s = SpuState::new();
    s.fpscr = rounding_fields(modes);
    s.regs[1] = from_doublewords(a);
    s.regs[3] = [0xAA; 16];
    let insn = crate::decode::decode(rr(op, 3, 1)).expect("decodes");
    assert_eq!(
        execute(&insn, &mut s, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
    (doublewords(s.regs[3]), s.fpscr & !rounding_fields(modes))
}

fn check_frds(a: [u64; 2], modes: [Mode; 2]) {
    let expected: [(u64, DFlags); 2] = std::array::from_fn(|i| frds_oracle(a[i], modes[i]));
    let (got, fpscr) = run(FRDS, a, modes);
    let context = format!("{modes:?} {a:016x?}");
    // [SPU-ISA p:224 s:9] the single result in the left word, zeros in the right.
    assert_eq!(got, expected.map(|(bits, _)| bits << 32), "{context}");
    assert_eq!(fpscr, fpscr_of(expected.map(|(_, f)| f)), "{context}");
}

/// Doubles at the single range's edges, the single denormal range,
/// ordinary values, halfway points and specials.
const DOUBLES: [u64; 20] = [
    0x0000_0000_0000_0000,
    0x000F_FFFF_FFFF_FFFF, // a double denormal
    0x0010_0000_0000_0000, // the smallest normal double, far below single
    0x36A0_0000_0000_0000, // 2^-149, the smallest single denormal
    0x3690_0000_0000_0000, // 2^-150, half of it
    0x3690_0000_0000_0001, // just above half of it
    0x380F_FFFF_E000_0000, // (1 - 2^-24) x 2^-126, tiny and halfway
    0x3810_0000_0000_0000, // 2^-126, the smallest normal single
    0x3FF0_0000_0000_0000, // 1
    0x3FF0_0000_1000_0000, // 1 + 2^-24, halfway, even below
    0x3FF0_0000_3000_0000, // 1 + 3 x 2^-24, halfway, odd below
    0x3FF0_0000_1000_0001, // just above halfway
    0x3FF0_0000_0FFF_FFFF, // just below halfway
    0x4009_21FB_5444_2D18, // pi
    0x47EF_FFFF_E000_0000, // the largest single
    0x47EF_FFFF_F000_0000, // halfway above the largest single
    0x47F0_0000_0000_0000, // 2^128
    0x7FF0_0000_0000_0000, // infinity
    0x7FF8_0000_0000_0000, // QNaN
    0x7FF0_0000_0000_0001, // SNaN
];

#[test]
fn frds_matches_the_oracle_in_every_mode() {
    for mode in MODES {
        for &d in &DOUBLES {
            check_frds([d, d | 1 << 63], [mode, mode]);
        }
    }
}

// [Verdonk2001Conversions p:124 s:3] every last, round and sticky combination at the 24-bit boundary.
#[test]
fn frds_rounds_every_position_like_the_oracle() {
    // A single significand's last bit even or odd, or all ones (a carry),
    // then the 29 dropped bits: zero, below, at and above half.
    let kept = [0u64, 1, 2, 0x7F_FFFF];
    let dropped = [0u64, 1, 0x0FFF_FFFF, 0x1000_0000, 0x1000_0001, 0x1FFF_FFFF];
    // Around 1, at the top of the single range, and in its denormal range.
    let binades = [
        0x3FF0_0000_0000_0000u64,
        0x47E0_0000_0000_0000,
        0x37D0_0000_0000_0000,
    ];
    for mode in MODES {
        for &binade in &binades {
            for &k in &kept {
                for &r in &dropped {
                    let d = binade | k << 29 | r;
                    check_frds([d, d | 1 << 63], [mode, mode]);
                }
            }
        }
    }
}

/// Host `as f32` rounds to nearest, so it is a second vote there, the
/// CBE's flushed denormal inputs and default NaN applied on top.
#[test]
fn frds_agrees_with_the_host_to_nearest() {
    for &d in &DOUBLES {
        for d in [d, d | 1 << 63] {
            let x = if d >> 52 & 0x7FF == 0 {
                f64::from_bits(d & 1 << 63)
            } else {
                f64::from_bits(d)
            };
            let single = x as f32;
            let want = if single.is_nan() {
                SINGLE_NAN
            } else {
                u64::from(single.to_bits())
            };
            assert_eq!(
                run(FRDS, [d, 0], [Mode::Nearest; 2]).0[0] >> 32,
                want,
                "{d:016x}"
            );
        }
    }
}

/// Host `as f32` again, stepped one ulp where a directed mode differs
/// from nearest: a vote for every mode that shares no code with the
/// oracle's rounder.
#[test]
fn frds_agrees_with_the_host_in_every_directed_mode() {
    let kept = [0u64, 1, 0x7F_FFFF];
    let dropped = [0u64, 1, 0x1000_0000, 0x1FFF_FFFF];
    let mut doubles: Vec<u64> = DOUBLES
        .into_iter()
        .filter(|d| (1..0x7FF).contains(&(d >> 52 & 0x7FF)))
        .collect();
    for b in [
        0x3FF0_0000_0000_0000u64,
        0x47E0_0000_0000_0000,
        0x37D0_0000_0000_0000,
    ] {
        for k in kept {
            for r in dropped {
                doubles.push(b | k << 29 | r);
            }
        }
    }
    for d in doubles {
        for d in [d, d | 1 << 63] {
            let x = f64::from_bits(d);
            let nearest = x as f32;
            // One ulp of magnitude away from or toward zero.
            let away = |r: f32| f32::from_bits(r.to_bits() + 1);
            let toward = |r: f32| f32::from_bits(r.to_bits() - 1);
            let above = f64::from(nearest) > x;
            let below = f64::from(nearest) < x;
            for (mode, want) in [
                (
                    Mode::Zero,
                    if f64::from(nearest).abs() > x.abs() {
                        toward(nearest)
                    } else {
                        nearest
                    },
                ),
                (
                    Mode::Up,
                    if !below {
                        nearest
                    } else if x > 0.0 {
                        away(nearest)
                    } else {
                        toward(nearest)
                    },
                ),
                (
                    Mode::Down,
                    if !above {
                        nearest
                    } else if x > 0.0 {
                        toward(nearest)
                    } else {
                        away(nearest)
                    },
                ),
            ] {
                let got = run(FRDS, [d, 0], [mode; 2]).0[0] >> 32;
                assert_eq!(got, u64::from(want.to_bits()), "{mode:?} {d:016x}");
            }
        }
    }
}

// [SPU-ISA p:199 s:9.2.2] UNF is tininess before rounding: a result that rounds up into the normal range still raises it.
#[test]
fn frds_underflow_is_tininess_before_rounding() {
    let slice = |offset: u32| fpscr_field(FPSCR_DOUBLE_FIRST[0] + offset, 1);
    let (got, fpscr) = run(FRDS, [0x380F_FFFF_E000_0000, 0], [Mode::Nearest; 2]);
    assert_eq!(got[0], 0x0080_0000 << 32);
    assert_eq!(fpscr, slice(1) | slice(2));
    // Overflow above the largest single, toward zero and to nearest.
    let (got, fpscr) = run(
        FRDS,
        [0x47F0_0000_0000_0000; 2],
        [Mode::Zero, Mode::Nearest],
    );
    assert_eq!(got, [0x7F7F_FFFF << 32, 0x7F80_0000 << 32]);
    assert_ne!(fpscr & slice(0), 0, "OVF");
}

// [SPU-ISA p:225 s:9] the left word converts and the right word is ignored.
#[test]
fn fesd_extends_the_left_word_and_ignores_the_right() {
    let words = [
        0x0000_0000u32,
        0x8000_0000,
        0x0000_0001, // a single denormal
        0x807F_FFFF,
        0x0080_0000,
        0x3F80_0000,
        0x4049_0FDB,
        0x7F7F_FFFF,
        0x7F80_0000, // infinity
        0xFF80_0000,
        0x7FC0_0000, // QNaN
        0xFF80_0001, // SNaN
    ];
    for &w in &words {
        let a = [u64::from(w) << 32 | 0xDEAD_BEEF, u64::from(w) << 32];
        let (got, fpscr) = run(FESD, a, [Mode::Up, Mode::Down]);
        let (bits, flags) = fesd_oracle(w);
        assert_eq!(got, [bits; 2], "{w:08x}");
        assert_eq!(fpscr, fpscr_of([flags; 2]), "{w:08x}");
    }
}

#[test]
#[ignore = "exhaustive over 2^32 single inputs; run with --release -- --ignored"]
fn fesd_matches_the_oracle_for_every_single() {
    let insn = crate::decode::decode(rr(FESD, 3, 1)).expect("decodes");
    let mut s = SpuState::new();
    for w in (0..=u32::MAX).step_by(2) {
        let pair = [w, w + 1];
        s.regs[1] = from_doublewords(pair.map(|w| u64::from(w) << 32 | u64::from(!w)));
        s.fpscr = 0;
        execute(&insn, &mut s, UnitId::new(0));
        let expected = pair.map(fesd_oracle);
        assert_eq!(
            doublewords(s.regs[3]),
            expected.map(|(bits, _)| bits),
            "{w:08x}"
        );
        assert_eq!(s.fpscr, fpscr_of(expected.map(|(_, f)| f)), "{w:08x}");
    }
}
