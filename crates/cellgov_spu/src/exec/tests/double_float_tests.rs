//! dfa, dfs and dfm against an integer IEEE 754 oracle with the CBE's
//! deviations, in all four rounding modes, with host `f64` as a second
//! vote in round to nearest.

use super::*;
use crate::exec::lanes::{doublewords, from_doublewords};
use crate::state::SpuState;
use cellgov_ps3_abi::hw::spu_fpscr::{fpscr_field, FPSCR_DOUBLE_FIRST, FPSCR_RN_FIRST};

// [SPU-ISA p:28 s:2.3] RR: 11-bit opcode, RB, RA, RT.
fn rr(op: u32, rt: u32, ra: u32, rb: u32) -> u32 {
    op << 21 | rb << 14 | ra << 7 | rt
}

const DFA: u32 = 0x2CC;
const DFS: u32 = 0x2CD;
const DFM: u32 = 0x2CE;

const DEFAULT_NAN: u64 = 0x7FF8_0000_0000_0000;

/// A rounding mode by its FPSCR code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Nearest = 0,
    Zero = 1,
    Up = 2,
    Down = 3,
}
const MODES: [Mode; 4] = [Mode::Nearest, Mode::Zero, Mode::Up, Mode::Down];

/// The oracle's six flags for one slice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct DFlags {
    overflow: bool,
    underflow: bool,
    inexact: bool,
    invalid: bool,
    nan: bool,
    denormal: bool,
}

/// A decoded double, a denormal already read as a zero of its sign.
#[derive(Debug, Clone, Copy)]
enum Val {
    Zero(bool),
    Fin { neg: bool, mag: u128, exp: i32 },
    Inf(bool),
    NaN { quiet: bool },
}

// [SPU-ISA p:199 s:9.2.2] a denormal operand reads as zero and sets DENORM; a NaN operand sets NaN.
fn decode(bits: u64, flags: &mut DFlags) -> Val {
    let neg = bits >> 63 == 1;
    let e = (bits >> 52 & 0x7FF) as i32;
    let f = bits & ((1 << 52) - 1);
    match e {
        0 => {
            flags.denormal |= f != 0;
            Val::Zero(neg)
        }
        0x7FF if f == 0 => Val::Inf(neg),
        0x7FF => {
            flags.nan = true;
            Val::NaN {
                quiet: f >> 51 == 1,
            }
        }
        _ => Val::Fin {
            neg,
            mag: u128::from(f | 1 << 52),
            exp: e - 1075,
        },
    }
}

/// Rounds `(-1)^neg * (mag + s) * 2^exp`, where `s` is in (0, 1) when
/// `sticky`, to a double in `mode`.
// [SPU-ISA p:199 s:9.2.2] UNF is a tiny result before rounding that is also inexact.
fn round(neg: bool, mag: u128, exp: i32, sticky: bool, mode: Mode) -> (u64, DFlags) {
    let sign = u64::from(neg) << 63;
    let top = 127 - mag.leading_zeros() as i32;
    let binade = exp + top;
    let tiny = binade < -1022;
    let lsb = binade.max(-1022) - 52;
    let (mut kept, rem, half) = if exp >= lsb {
        (mag << (exp - lsb), 0u128, 1u128)
    } else {
        let d = (lsb - exp) as u32;
        if d >= 127 {
            (0, mag, u128::MAX)
        } else {
            (mag >> d, mag & ((1 << d) - 1), 1 << (d - 1))
        }
    };
    let inexact = rem != 0 || sticky;
    let above = rem > half || (rem == half && sticky);
    let tie = rem == half && !sticky;
    let up = match mode {
        Mode::Nearest => above || (tie && kept & 1 == 1),
        Mode::Zero => false,
        Mode::Up => inexact && !neg,
        Mode::Down => inexact && neg,
    };
    kept += u128::from(up);
    let mut biased = binade + 1023;
    if !tiny && kept >> 53 != 0 {
        kept >>= 1;
        biased += 1;
    }
    let flags = DFlags {
        inexact,
        underflow: tiny && inexact,
        ..DFlags::default()
    };
    if !tiny && biased >= 0x7FF {
        let to_max = match mode {
            Mode::Nearest => false,
            Mode::Zero => true,
            Mode::Up => neg,
            Mode::Down => !neg,
        };
        let bits = if to_max {
            0x7FEF_FFFF_FFFF_FFFF
        } else {
            0x7FF0_0000_0000_0000
        };
        return (
            sign | bits,
            DFlags {
                overflow: true,
                inexact: true,
                ..flags
            },
        );
    }
    let field = if tiny {
        // A carry into bit 52 is the smallest normal, which this encodes.
        kept as u64
    } else {
        (biased as u64) << 52 | (kept as u64 & ((1 << 52) - 1))
    };
    (sign | field, flags)
}

/// The oracle's result for one slot.
fn oracle(op: u32, a: u64, b: u64, mode: Mode) -> (u64, DFlags) {
    let mut flags = DFlags::default();
    let x = decode(a, &mut flags);
    let y = decode(b, &mut flags);
    let (bits, more) = compute(op, x, y, mode);
    (
        bits,
        DFlags {
            overflow: more.overflow,
            underflow: more.underflow,
            inexact: more.inexact,
            invalid: more.invalid,
            ..flags
        },
    )
}

fn compute(op: u32, x: Val, y: Val, mode: Mode) -> (u64, DFlags) {
    let invalid = DFlags {
        invalid: true,
        ..DFlags::default()
    };
    let none = DFlags::default();
    // [SPU-ISA p:197 s:9.2] every NaN result is the default QNaN.
    if matches!(x, Val::NaN { .. }) || matches!(y, Val::NaN { .. }) {
        let signaling =
            matches!(x, Val::NaN { quiet: false }) || matches!(y, Val::NaN { quiet: false });
        return (
            DEFAULT_NAN,
            DFlags {
                invalid: signaling,
                ..none
            },
        );
    }
    let neg_of = |v: Val| match v {
        Val::Zero(n) | Val::Inf(n) => n,
        Val::Fin { neg, .. } => neg,
        Val::NaN { .. } => false,
    };
    let inf = |n: bool| u64::from(n) << 63 | 0x7FF0_0000_0000_0000;
    let y = if op == DFS {
        match y {
            Val::Zero(n) => Val::Zero(!n),
            Val::Inf(n) => Val::Inf(!n),
            Val::Fin { neg, mag, exp } => Val::Fin {
                neg: !neg,
                mag,
                exp,
            },
            nan => nan,
        }
    } else {
        y
    };
    if op == DFM {
        return match (x, y) {
            (Val::Inf(_), Val::Zero(_)) | (Val::Zero(_), Val::Inf(_)) => (DEFAULT_NAN, invalid),
            (Val::Inf(_), _) | (_, Val::Inf(_)) => (inf(neg_of(x) != neg_of(y)), none),
            (
                Val::Fin {
                    mag: m1, exp: e1, ..
                },
                Val::Fin {
                    mag: m2, exp: e2, ..
                },
            ) => round(neg_of(x) != neg_of(y), m1 * m2, e1 + e2, false, mode),
            _ => (u64::from(neg_of(x) != neg_of(y)) << 63, none),
        };
    }
    match (x, y) {
        (Val::Inf(p), Val::Inf(q)) if p != q => (DEFAULT_NAN, invalid),
        (Val::Inf(p), _) | (_, Val::Inf(p)) => (inf(p), none),
        (Val::Zero(p), Val::Zero(q)) if p == q => (u64::from(p) << 63, none),
        (Val::Zero(_), Val::Zero(_)) => (u64::from(mode == Mode::Down) << 63, none),
        (Val::Zero(_), Val::Fin { neg, mag, exp }) | (Val::Fin { neg, mag, exp }, Val::Zero(_)) => {
            round(neg, mag, exp, false, mode)
        }
        (
            Val::Fin {
                neg: n1,
                mag: m1,
                exp: e1,
            },
            Val::Fin {
                neg: n2,
                mag: m2,
                exp: e2,
            },
        ) => {
            let ((bn, bm, be), (sn, sm, se)) = if e1 >= e2 {
                ((n1, m1, e1), (n2, m2, e2))
            } else {
                ((n2, m2, e2), (n1, m1, e1))
            };
            let signed = |n: bool, v: i128| if n { -v } else { v };
            let (value, exp, sticky) = if se + 53 < be - 8 {
                // The smaller value lies strictly below one unit 8 bits under
                // the larger's lowest bit, so it only nudges the sum.
                let big = (bm << 8) as i128;
                (
                    signed(bn, big) - i128::from(bn != sn) * signed(bn, 1),
                    be - 8,
                    true,
                )
            } else {
                let e = be.min(se);
                (
                    signed(bn, (bm << (be - e)) as i128) + signed(sn, (sm << (se - e)) as i128),
                    e,
                    false,
                )
            };
            if value == 0 {
                // An exact zero sum is +0, or -0 under round toward -inf.
                return (u64::from(mode == Mode::Down) << 63, none);
            }
            round(value < 0, value.unsigned_abs(), exp, sticky, mode)
        }
        _ => unreachable!("NaN handled above"),
    }
}

/// The FPSCR bits two slices of oracle flags set.
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

/// The FPSCR holding `modes` in RN0 and RN1.
fn rounding_fields(modes: [Mode; 2]) -> u128 {
    FPSCR_RN_FIRST
        .into_iter()
        .zip(modes)
        .fold(0, |fpscr, (first, mode)| {
            fpscr | (mode as u128) << (128 - first - 2)
        })
}

/// Runs `op` with `modes` in RN0 / RN1; returns RT's doublewords and the
/// flag bits the operation set.
fn run(op: u32, a: [u64; 2], b: [u64; 2], modes: [Mode; 2]) -> ([u64; 2], u128) {
    let mut s = SpuState::new();
    s.fpscr = rounding_fields(modes);
    s.regs[1] = from_doublewords(a);
    s.regs[2] = from_doublewords(b);
    s.regs[3] = [0xAA; 16];
    let insn = crate::decode::decode(rr(op, 3, 1, 2)).expect("decodes");
    assert_eq!(
        execute(&insn, &mut s, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
    (doublewords(s.regs[3]), s.fpscr & !rounding_fields(modes))
}

fn check(op: u32, a: [u64; 2], b: [u64; 2], modes: [Mode; 2]) {
    let expected: [(u64, DFlags); 2] = std::array::from_fn(|i| oracle(op, a[i], b[i], modes[i]));
    let (got, fpscr) = run(op, a, b, modes);
    let context = format!("op {op:#x} {modes:?} {a:016x?} {b:016x?}");
    assert_eq!(got, expected.map(|(bits, _)| bits), "{context}");
    assert_eq!(fpscr, fpscr_of(expected.map(|(_, f)| f)), "{context}");
}

/// Zeros, denormals, the normal range's ends, ordinary values, infinities
/// and both NaN kinds.
const CLASSES: [u64; 14] = [
    0x0000_0000_0000_0000,
    0x0000_0000_0000_0001,
    0x000F_FFFF_FFFF_FFFF,
    0x0010_0000_0000_0000,
    0x0018_0000_0000_0000,
    0x3CA0_0000_0000_0000,
    0x3FF0_0000_0000_0000,
    0x3FF0_0000_0000_0001,
    0x3FF8_0000_0000_0000,
    0x4009_21FB_5444_2D18,
    0x7FEF_FFFF_FFFF_FFFF,
    0x7FF0_0000_0000_0000,
    0x7FF8_0000_0000_0000,
    0x7FF0_0000_0000_0001,
];

#[test]
fn every_class_pair_matches_the_oracle_in_every_mode() {
    let signed: Vec<u64> = CLASSES.iter().flat_map(|&c| [c, c | 1 << 63]).collect();
    for op in [DFA, DFS, DFM] {
        for mode in MODES {
            for &a in &signed {
                for pair in signed.chunks(2) {
                    check(op, [a, a], [pair[0], pair[1]], [mode, mode]);
                }
            }
        }
    }
}

/// Host `f64` as a second vote in round to nearest, the CBE's flushed
/// denormal operands and default NaN applied on top.
#[test]
fn round_to_nearest_agrees_with_the_host() {
    let signed: Vec<u64> = CLASSES.iter().flat_map(|&c| [c, c | 1 << 63]).collect();
    let flush = |bits: u64| {
        if bits >> 52 & 0x7FF == 0 {
            bits & 1 << 63
        } else {
            bits
        }
    };
    for op in [DFA, DFS, DFM] {
        for &a in &signed {
            for &b in &signed {
                let (x, y) = (f64::from_bits(flush(a)), f64::from_bits(flush(b)));
                let host = match op {
                    DFA => x + y,
                    DFS => x - y,
                    _ => x * y,
                };
                let want = if host.is_nan() {
                    DEFAULT_NAN
                } else {
                    host.to_bits()
                };
                let (got, _) = run(op, [a, a], [b, b], [Mode::Nearest; 2]);
                assert_eq!(got[0], want, "op {op:#x} {a:016x} {b:016x}");
            }
        }
    }
}

// [Verdonk2001Basic p:104 s:4.2] every last, round and sticky combination, with and without a carry.
#[test]
fn every_rounding_position_matches_the_oracle() {
    // 1.0 (even last bit) and 1 + ulp (odd), plus 2 - ulp, which carries.
    let bases = [
        0x3FF0_0000_0000_0000u64,
        0x3FF0_0000_0000_0001,
        0x3FFF_FFFF_FFFF_FFFF,
    ];
    // A quarter, a half, just above a half and three quarters of an ulp of
    // 1.0, and a tiny sticky-only amount.
    let tails = [
        0x3C90_0000_0000_0000u64, // 2^-54
        0x3CA0_0000_0000_0000,    // 2^-53
        0x3CA0_0000_0000_0001,    // just above 2^-53
        0x3CA8_0000_0000_0000,    // 3 x 2^-54
        0x3960_0000_0000_0000,    // 2^-105
    ];
    for mode in MODES {
        for &a in &bases {
            for &t in &tails {
                for sign in [0, 1u64 << 63] {
                    check(DFA, [a | sign, a | sign], [t | sign, t], [mode, mode]);
                    check(DFS, [a | sign, a], [t | sign, t], [mode, mode]);
                }
            }
        }
        // Products into the denormal range, onto the normal boundary, and
        // onto the overflow boundary.
        for i in 0..8u64 {
            let a = 0x3FF0_0000_0000_0000 + i * 0x0000_1000_0000_0001;
            for b in [
                0x0010_0000_0000_0000u64,
                0x0008_0000_0000_0000 | 0x0010_0000_0000_0000,
                0x0010_0000_0000_0003,
                0x0340_0000_0000_0001,
                0x7FE0_0000_0000_0001,
                0x7FEF_FFFF_FFFF_FFFF,
            ] {
                check(DFM, [a, a | 1 << 63], [b, b], [mode, mode]);
                // a / 2 is below 1, so its products with the small b are tiny.
                check(
                    DFM,
                    [a - 0x0010_0000_0000_0000, a],
                    [b, b | 1 << 63],
                    [mode, mode],
                );
            }
        }
        // The largest value plus a quarter, a half, just above a half, one
        // and just above one of its ulp (2^971).
        for t in [
            0x7C80_0000_0000_0000u64,
            0x7C90_0000_0000_0000,
            0x7C90_0000_0000_0001,
            0x7CA0_0000_0000_0000,
            0x7CA0_0000_0000_0001,
        ] {
            check(
                DFA,
                [0x7FEF_FFFF_FFFF_FFFF; 2],
                [t, t | 1 << 63],
                [mode, mode],
            );
        }
    }
}

// [SPU-ISA p:199 s:9.2.2] UNF is tininess before rounding together with an inexact result.
#[test]
fn underflow_is_tininess_before_rounding() {
    let [unf, inx] = [1, 2].map(|offset| fpscr_field(FPSCR_DOUBLE_FIRST[0] + offset, 1));
    // (1 - 2^-53) x 2^-1022 is tiny and halfway between two denormals; to
    // nearest it rounds up to the smallest normal and still raises UNF.
    let (got, fpscr) = run(
        DFM,
        [0x3FEF_FFFF_FFFF_FFFF, 0],
        [0x0010_0000_0000_0000, 0],
        [Mode::Nearest; 2],
    );
    assert_eq!(got[0], 0x0010_0000_0000_0000);
    assert_eq!(fpscr, unf | inx);
    let (got, fpscr) = run(
        DFM,
        [0x3FEF_FFFF_FFFF_FFFF, 0],
        [0x0010_0000_0000_0000, 0],
        [Mode::Zero; 2],
    );
    assert_eq!(got[0], 0x000F_FFFF_FFFF_FFFF);
    assert_eq!(fpscr, unf | inx);
    // Toward +inf it carries into the smallest normal; toward -inf it stays
    // the largest denormal. Both are tiny before rounding.
    for (mode, want) in [
        (Mode::Up, 0x0010_0000_0000_0000),
        (Mode::Down, 0x000F_FFFF_FFFF_FFFF),
    ] {
        let (got, fpscr) = run(
            DFM,
            [0x3FEF_FFFF_FFFF_FFFF, 0],
            [0x0010_0000_0000_0000, 0],
            [mode; 2],
        );
        assert_eq!((got[0], fpscr), (want, unf | inx), "{mode:?}");
    }
    // Exactly the smallest normal: neither.
    let (_, fpscr) = run(
        DFM,
        [0x3FF0_0000_0000_0000, 0],
        [0x0010_0000_0000_0000, 0],
        [Mode::Nearest; 2],
    );
    assert_eq!(fpscr, 0);
    // Not tiny before rounding, inexact, truncated down to the smallest
    // normal: INX without UNF.
    let (got, fpscr) = run(
        DFM,
        [0x3FF0_0000_0000_0001, 0],
        [0x0010_0000_0000_0001, 0],
        [Mode::Zero; 2],
    );
    assert_eq!(got[0], 0x0010_0000_0000_0002);
    assert_eq!(fpscr, inx);
}

// [SPU-ISA p:197 s:9.2] the default QNaN for every NaN result; [SPU-ISA p:199 s:9.2.2] INV for an SNaN, infinity less infinity and infinity times zero, NaN for a NaN operand, DENORM for a denormal operand.
#[test]
fn the_special_cases_follow_the_cbe() {
    let slice = |offset: u32| fpscr_field(FPSCR_DOUBLE_FIRST[0] + offset, 1);
    let (inv, nan, denorm) = (slice(3), slice(4), slice(5));
    // x + (-x): +0, except -0 when rounding toward -inf.
    for mode in MODES {
        let (got, _) = run(
            DFA,
            [0x3FF0_0000_0000_0000; 2],
            [0xBFF0_0000_0000_0000; 2],
            [mode; 2],
        );
        let want = if mode == Mode::Down { 1 << 63 } else { 0 };
        assert_eq!(got, [want; 2], "{mode:?}");
    }
    // Infinity less infinity, and infinity times zero.
    let (got, fpscr) = run(
        DFS,
        [0x7FF0_0000_0000_0000, 0],
        [0x7FF0_0000_0000_0000, 0],
        [Mode::Nearest; 2],
    );
    assert_eq!((got[0], fpscr), (DEFAULT_NAN, inv));
    let (got, fpscr) = run(
        DFM,
        [0xFFF0_0000_0000_0000, 0],
        [0x8000_0000_0000_0000, 0],
        [Mode::Nearest; 2],
    );
    assert_eq!((got[0], fpscr), (DEFAULT_NAN, inv));
    // An SNaN raises INV and NaN; a QNaN raises NaN. Neither propagates,
    // and negative NaN operands still give the positive default QNaN.
    let (got, fpscr) = run(
        DFA,
        [0xFFF0_0000_0000_0001, 0],
        [0x3FF0_0000_0000_0000, 0],
        [Mode::Nearest; 2],
    );
    assert_eq!((got[0], fpscr), (DEFAULT_NAN, inv | nan));
    let (got, fpscr) = run(
        DFM,
        [0xFFF8_0000_0000_1234, 0],
        [0xFFF8_0000_0000_0000, 0],
        [Mode::Nearest; 2],
    );
    assert_eq!((got[0], fpscr), (DEFAULT_NAN, nan));
    // A denormal operand is a zero of its sign and raises DENORM only.
    let (got, fpscr) = run(
        DFM,
        [0x800F_FFFF_FFFF_FFFF, 0],
        [0x3FF0_0000_0000_0000, 0],
        [Mode::Nearest; 2],
    );
    assert_eq!((got[0], fpscr), (1 << 63, denorm));
    let (got, fpscr) = run(
        DFA,
        [0x0000_0000_0000_0001, 0],
        [0x3FF0_0000_0000_0000, 0],
        [Mode::Nearest; 2],
    );
    assert_eq!((got[0], fpscr), (0x3FF0_0000_0000_0000, denorm));
}

// [SPU-ISA p:197 s:9.2] slice 0 rounds by RN0 and slice 1 by RN1; [SPU-ISA p:200 s:9.3] each slice has its own flags.
#[test]
fn each_slice_rounds_by_its_own_mode_and_keeps_its_own_flags() {
    // 1 + 2^-60 rounds up toward +inf and down toward -inf.
    let (got, fpscr) = run(
        DFA,
        [0x3FF0_0000_0000_0000; 2],
        [0x3C30_0000_0000_0000; 2],
        [Mode::Up, Mode::Down],
    );
    assert_eq!(got, [0x3FF0_0000_0000_0001, 0x3FF0_0000_0000_0000]);
    let inx = |slice: usize| fpscr_field(FPSCR_DOUBLE_FIRST[slice] + 2, 1);
    assert_eq!(fpscr, inx(0) | inx(1));
    // Only slice 1 is inexact.
    let (_, fpscr) = run(
        DFA,
        [0x3FF0_0000_0000_0000; 2],
        [0x3FF0_0000_0000_0000, 0x3C30_0000_0000_0000],
        [Mode::Nearest; 2],
    );
    assert_eq!(fpscr, inx(1));
}
