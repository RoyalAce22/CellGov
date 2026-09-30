//! dfma, dfms, dfnms and dfnma against an exact oracle in all four modes,
//! with host `f64::mul_add`, rounded once, as a second vote in round to
//! nearest.

use super::*;
use crate::exec::lanes::{doublewords, from_doublewords};
use crate::state::SpuState;
use cellgov_ps3_abi::hw::spu_fpscr::{fpscr_field, FPSCR_DOUBLE_FIRST, FPSCR_RN_FIRST};

/// [SPU-ISA p:28 s:2.3] RR: 11-bit opcode, RB, RA, RT.
fn rr(op: u32, rt: u32, ra: u32, rb: u32) -> u32 {
    op << 21 | rb << 14 | ra << 7 | rt
}

const DFMA: u32 = 0x35C;
const DFMS: u32 = 0x35D;
const DFNMS: u32 = 0x35E;
const DFNMA: u32 = 0x35F;
const FORMS: [u32; 4] = [DFMA, DFMS, DFNMS, DFNMA];

const DEFAULT_NAN: u64 = 0x7FF8_0000_0000_0000;

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

/// A 256-bit magnitude, `hi` the upper half.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct U256 {
    hi: u128,
    lo: u128,
}

impl U256 {
    fn from(x: u128) -> Self {
        U256 { hi: 0, lo: x }
    }
    fn shl(self, n: u32) -> Self {
        match n {
            0 => self,
            1..=127 => U256 {
                hi: self.hi << n | self.lo >> (128 - n),
                lo: self.lo << n,
            },
            _ => U256 {
                hi: self.lo << (n - 128),
                lo: 0,
            },
        }
    }
    fn add(self, o: Self) -> Self {
        let (lo, carry) = self.lo.overflowing_add(o.lo);
        U256 {
            hi: self.hi + o.hi + u128::from(carry),
            lo,
        }
    }
    fn sub(self, o: Self) -> Self {
        let (lo, borrow) = self.lo.overflowing_sub(o.lo);
        U256 {
            hi: self.hi - o.hi - u128::from(borrow),
            lo,
        }
    }
    fn is_zero(self) -> bool {
        self.hi == 0 && self.lo == 0
    }
    /// The index of the top set bit.
    fn top(self) -> i32 {
        if self.hi != 0 {
            255 - self.hi.leading_zeros() as i32
        } else {
            127 - self.lo.leading_zeros() as i32
        }
    }
    fn bit(self, i: u32) -> bool {
        if i >= 128 {
            self.hi >> (i - 128) & 1 == 1
        } else {
            self.lo >> i & 1 == 1
        }
    }
    /// Any bit below `n` set.
    fn any_below(self, n: u32) -> bool {
        match n {
            0 => false,
            1..=127 => self.lo & ((1 << n) - 1) != 0,
            128 => self.lo != 0,
            _ => self.lo != 0 || self.hi & ((1u128 << (n - 128)) - 1) != 0,
        }
    }
    /// `self >> n`, which the caller knows fits in 128 bits.
    fn shr_small(self, n: u32) -> u128 {
        match n {
            0 => self.lo,
            1..=127 => self.lo >> n | self.hi << (128 - n),
            128..=255 => self.hi >> (n - 128),
            _ => 0,
        }
    }
}

/// A decoded double, a denormal already read as a zero of its sign.
#[derive(Debug, Clone, Copy)]
enum Val {
    Zero(bool),
    Fin { neg: bool, mag: u128, exp: i32 },
    Inf(bool),
    NaN { quiet: bool },
}

impl Val {
    fn neg(self) -> bool {
        match self {
            Val::Zero(n) | Val::Inf(n) => n,
            Val::Fin { neg, .. } => neg,
            Val::NaN { .. } => false,
        }
    }
    fn negated(self) -> Val {
        match self {
            Val::Zero(n) => Val::Zero(!n),
            Val::Inf(n) => Val::Inf(!n),
            Val::Fin { neg, mag, exp } => Val::Fin {
                neg: !neg,
                mag,
                exp,
            },
            nan => nan,
        }
    }
}

/// [SPU-ISA p:199 s:9.2.2] a denormal operand reads as zero with DENORM; a NaN operand sets NaN.
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

/// Rounds `(-1)^neg * (mag + s) * 2^exp`, `s` in (0, 1) when `sticky`.
///
/// [SPU-ISA p:199 s:9.2.2] UNF is tininess before rounding together with an inexact result.
fn round(neg: bool, mag: U256, exp: i32, sticky: bool, mode: Mode) -> (u64, DFlags) {
    let sign = u64::from(neg) << 63;
    let binade = exp + mag.top();
    let tiny = binade < -1022;
    let lsb = binade.max(-1022) - 52;
    let (mut kept, guard, lower) = if exp >= lsb {
        (mag.shl((exp - lsb) as u32).lo, false, false)
    } else {
        let d = (lsb - exp) as u32;
        if d > 256 {
            (0, false, !mag.is_zero())
        } else {
            (mag.shr_small(d), mag.bit(d - 1), mag.any_below(d - 1))
        }
    };
    let lower = lower || sticky;
    let inexact = guard || lower;
    let up = match mode {
        Mode::Nearest => guard && (lower || kept & 1 == 1),
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
        kept as u64
    } else {
        (biased as u64) << 52 | (kept as u64 & ((1 << 52) - 1))
    };
    (sign | field, flags)
}

/// `(pn, pm, pe) + (zn, zm, ze)` rounded, for nonzero magnitudes.
fn round_sum(p: (bool, u128, i32), z: (bool, u128, i32), mode: Mode) -> (u64, DFlags) {
    let top = |(_, m, e): (bool, u128, i32)| e + 127 - m.leading_zeros() as i32;
    let (big, small) = if top(p) >= top(z) { (p, z) } else { (z, p) };
    let (bn, bm, be) = big;
    let (sn, sm, se) = small;
    let (value_neg, value, exp, sticky) = if top(small) < top(big) - 120 {
        // The smaller value lies strictly below one unit at a scale 120
        // bits under the larger's top, below every bit the result keeps.
        let floor = top(big) - 120;
        let scaled = U256::from(bm).shl((be - floor) as u32);
        let value = if bn == sn {
            scaled
        } else {
            scaled.sub(U256::from(1))
        };
        (bn, value, floor, true)
    } else {
        let floor = be.min(se);
        let b = U256::from(bm).shl((be - floor) as u32);
        let s = U256::from(sm).shl((se - floor) as u32);
        if bn == sn {
            (bn, b.add(s), floor, false)
        } else if b >= s {
            (bn, b.sub(s), floor, false)
        } else {
            (sn, s.sub(b), floor, false)
        }
    };
    if value.is_zero() {
        return (u64::from(mode == Mode::Down) << 63, DFlags::default());
    }
    round(value_neg, value, exp, sticky, mode)
}

/// The oracle's result for one slot.
fn oracle(op: u32, a: u64, b: u64, c: u64, mode: Mode) -> (u64, DFlags) {
    let mut flags = DFlags::default();
    let x = decode(a, &mut flags);
    let y = decode(b, &mut flags);
    let z = decode(c, &mut flags);
    let z = if matches!(op, DFMS | DFNMS) {
        z.negated()
    } else {
        z
    };
    let (bits, more) = fused(x, y, z, mode);
    let is_nan = bits & !(1 << 63) > 0x7FF0_0000_0000_0000;
    // [SPU-ISA p:211 s:9] and [SPU-ISA p:214 s:9]: the negated forms negate every result that is not a NaN.
    let bits = if matches!(op, DFNMS | DFNMA) && !is_nan {
        bits ^ 1 << 63
    } else {
        bits
    };
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

fn fused(x: Val, y: Val, z: Val, mode: Mode) -> (u64, DFlags) {
    let none = DFlags::default();
    let invalid = DFlags {
        invalid: true,
        ..none
    };
    let is_nan = |v: Val| matches!(v, Val::NaN { .. });
    if is_nan(x) || is_nan(y) || is_nan(z) {
        let signaling = [x, y, z]
            .iter()
            .any(|v| matches!(v, Val::NaN { quiet: false }));
        return (
            DEFAULT_NAN,
            DFlags {
                invalid: signaling,
                ..none
            },
        );
    }
    let inf = |n: bool| u64::from(n) << 63 | 0x7FF0_0000_0000_0000;
    let product_neg = x.neg() != y.neg();
    match (x, y) {
        (Val::Inf(_), Val::Zero(_)) | (Val::Zero(_), Val::Inf(_)) => return (DEFAULT_NAN, invalid),
        (Val::Inf(_), _) | (_, Val::Inf(_)) => {
            return match z {
                Val::Inf(n) if n != product_neg => (DEFAULT_NAN, invalid),
                _ => (inf(product_neg), none),
            };
        }
        _ => {}
    }
    if let Val::Inf(n) = z {
        return (inf(n), none);
    }
    let product = match (x, y) {
        (
            Val::Fin {
                mag: m1, exp: e1, ..
            },
            Val::Fin {
                mag: m2, exp: e2, ..
            },
        ) => Some((product_neg, m1 * m2, e1 + e2)),
        _ => None,
    };
    let addend = match z {
        Val::Fin { neg, mag, exp } => Some((neg, mag, exp)),
        _ => None,
    };
    match (product, addend) {
        (Some(p), Some(a)) => round_sum(p, a, mode),
        (Some((n, m, e)), None) | (None, Some((n, m, e))) => {
            round(n, U256::from(m), e, false, mode)
        }
        (None, None) => {
            // Two zeros: their common sign, else +0 (-0 toward -inf).
            let negative = if product_neg == z.neg() {
                product_neg
            } else {
                mode == Mode::Down
            };
            (u64::from(negative) << 63, none)
        }
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

/// Runs `op` with RA = `a`, RB = `b`, RT = `c`; returns RT and the flag bits.
fn run(op: u32, a: [u64; 2], b: [u64; 2], c: [u64; 2], modes: [Mode; 2]) -> ([u64; 2], u128) {
    let mut s = SpuState::new();
    s.fpscr = rounding_fields(modes);
    s.regs[1] = from_doublewords(a);
    s.regs[2] = from_doublewords(b);
    s.regs[3] = from_doublewords(c);
    let insn = crate::decode::decode(rr(op, 3, 1, 2)).expect("decodes");
    assert_eq!(
        execute(&insn, &mut s, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
    (doublewords(s.regs[3]), s.fpscr & !rounding_fields(modes))
}

fn check(op: u32, a: [u64; 2], b: [u64; 2], c: [u64; 2], modes: [Mode; 2]) {
    let expected: [(u64, DFlags); 2] =
        std::array::from_fn(|i| oracle(op, a[i], b[i], c[i], modes[i]));
    let (got, fpscr) = run(op, a, b, c, modes);
    let context = format!("op {op:#x} {modes:?} {a:016x?} {b:016x?} {c:016x?}");
    assert_eq!(got, expected.map(|(bits, _)| bits), "{context}");
    assert_eq!(fpscr, fpscr_of(expected.map(|(_, f)| f)), "{context}");
}

const CLASSES: [u64; 12] = [
    0x0000_0000_0000_0000,
    0x000F_FFFF_FFFF_FFFF,
    0x0010_0000_0000_0000,
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
fn every_class_triple_matches_the_oracle_in_every_mode() {
    let signed: Vec<u64> = CLASSES.iter().flat_map(|&c| [c, c | 1 << 63]).collect();
    for op in FORMS {
        for mode in MODES {
            for &a in &signed {
                for &b in &signed {
                    for pair in signed.chunks(2) {
                        check(op, [a, a], [b, b], [pair[0], pair[1]], [mode, mode]);
                    }
                }
            }
        }
    }
}

/// Host `f64::mul_add` rounds once, so it is a second vote in round to
/// nearest, the CBE's flushed denormals and default NaN applied on top.
#[test]
fn round_to_nearest_agrees_with_the_host_fused_multiply_add() {
    let values: Vec<u64> = CLASSES
        .iter()
        .flat_map(|&c| [c, c | 1 << 63])
        .chain([
            0x3FF0_0000_0000_0003,
            0xBFF0_0000_0000_0005,
            0x3CB0_0000_0000_0001,
            0x4340_0000_0000_0001,
        ])
        .collect();
    let flush = |bits: u64| {
        if bits >> 52 & 0x7FF == 0 {
            f64::from_bits(bits & 1 << 63)
        } else {
            f64::from_bits(bits)
        }
    };
    for op in FORMS {
        for &a in &values {
            for &b in &values {
                for &c in &values {
                    let (x, y, z) = (flush(a), flush(b), flush(c));
                    let host = match op {
                        DFMA => x.mul_add(y, z),
                        DFMS => x.mul_add(y, -z),
                        DFNMS => -x.mul_add(y, -z),
                        _ => -x.mul_add(y, z),
                    };
                    let want = if host.is_nan() {
                        DEFAULT_NAN
                    } else {
                        host.to_bits()
                    };
                    let (got, _) = run(op, [a, a], [b, b], [c, c], [Mode::Nearest; 2]);
                    assert_eq!(got[0], want, "op {op:#x} {a:016x} {b:016x} {c:016x}");
                }
            }
        }
    }
}

/// [SPU-ISA p:209 s:9] the multiplication is exact and not subject to limits on its range.
#[test]
fn the_product_is_not_rounded_before_the_add() {
    // (1 + 2^-52)^2 = 1 + 2^-51 + 2^-104; less 1 + 2^-51 leaves 2^-104,
    // which a product rounded first would lose.
    let a = [0x3FF0_0000_0000_0001; 2];
    let (got, fpscr) = run(DFMA, a, a, [0xBFF0_0000_0000_0002; 2], [Mode::Nearest; 2]);
    assert_eq!((got, fpscr), ([0x3970_0000_0000_0000; 2], 0));
    let (got, _) = run(DFMS, a, a, [0x3FF0_0000_0000_0002; 2], [Mode::Nearest; 2]);
    assert_eq!(got, [0x3970_0000_0000_0000; 2]);
    let (got, _) = run(DFNMS, a, a, [0x3FF0_0000_0000_0002; 2], [Mode::Nearest; 2]);
    assert_eq!(got, [0xB970_0000_0000_0000; 2]);
    // A product far above the double range brought back by the addend.
    let (got, _) = run(
        DFMA,
        [0x7FEF_FFFF_FFFF_FFFF; 2],
        [0x4000_0000_0000_0000; 2],
        [0xFFEF_FFFF_FFFF_FFFF; 2],
        [Mode::Nearest; 2],
    );
    assert_eq!(got, [0x7FEF_FFFF_FFFF_FFFF; 2]);
}

/// [SPU-ISA p:214 s:9] dfnma negates the rounded dfma result, so the direction applies before the sign flip.
#[test]
fn the_negated_forms_round_before_they_negate() {
    // (1 + 2^-52)^2 + 0 is 1 + 2^-51 + 2^-104, between two doubles.
    let a = [0x3FF0_0000_0000_0001; 2];
    let (got, _) = run(DFNMA, a, a, [0; 2], [Mode::Up, Mode::Down]);
    assert_eq!(got, [0xBFF0_0000_0000_0003, 0xBFF0_0000_0000_0002]);
    let (got, _) = run(DFNMS, a, a, [0; 2], [Mode::Zero, Mode::Nearest]);
    assert_eq!(got, [0xBFF0_0000_0000_0002, 0xBFF0_0000_0000_0002]);
    // A zero result negates too: -(+0) is -0.
    let (got, _) = run(
        DFNMA,
        [0x3FF0_0000_0000_0000; 2],
        [0x3FF0_0000_0000_0000; 2],
        [0xBFF0_0000_0000_0000; 2],
        [Mode::Nearest; 2],
    );
    assert_eq!(got, [1 << 63; 2]);
}

/// [SPU-ISA p:211 s:9] and [SPU-ISA p:214 s:9]: a QNaN result has sign bit 0.
#[test]
fn a_nan_result_keeps_sign_0_through_the_negated_forms() {
    for op in [DFNMS, DFNMA] {
        let (got, _) = run(
            op,
            [0xFFF8_0000_0000_0000, 0x7FF0_0000_0000_0000],
            [0x3FF0_0000_0000_0000, 0],
            [0xFFF8_0000_0000_0001, 0x3FF0_0000_0000_0000],
            [Mode::Nearest; 2],
        );
        assert_eq!(got, [DEFAULT_NAN; 2], "op {op:#x}");
    }
}

#[test]
fn rt_is_read_before_it_is_written() {
    // RT is also RA: 2 x 3 + 2 = 8.
    let mut s = SpuState::new();
    s.regs[5] = from_doublewords([0x4000_0000_0000_0000; 2]);
    s.regs[6] = from_doublewords([0x4008_0000_0000_0000; 2]);
    let insn = crate::decode::decode(rr(DFMA, 5, 5, 6)).expect("decodes");
    execute(&insn, &mut s, UnitId::new(0));
    assert_eq!(doublewords(s.regs[5]), [0x4020_0000_0000_0000; 2]);
}
