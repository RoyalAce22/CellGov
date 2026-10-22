//! The double-precision and IEEE model: the double arithmetic, frds,
//! and fesd.

use cellgov_ps3_abi::hw::spu_fpscr::FPSCR_DOUBLE_FIRST;
use cellgov_ps3_abi::hw::spu_isa::SPU_DEFAULT_QNAN_F64;

use super::effect::{push_write, Effect, Flag, FlagWrite, Insn, Known, Op, CLEAR, SET};
use crate::spu_documented::dyadic::{round, Dyadic, Mode, DOUBLE, SINGLE};
use crate::spu_documented::question::Question;

/// The six double-precision flags of one slot, in FPSCR order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DoubleFlags(Vec<(usize, Flag)>);

impl DoubleFlags {
    const NAMES: [&'static str; 6] = ["ovf", "unf", "inx", "inv", "nan", "denorm"];

    fn new(flags: [Flag; 6]) -> Self {
        Self(flags.into_iter().enumerate().collect())
    }

    fn uniform(flag: Flag) -> Self {
        Self::new([flag; 6])
    }

    fn set(&mut self, index: usize, flag: Flag) {
        if let Some(entry) = self.0.iter_mut().find(|(i, _)| *i == index) {
            entry.1 = flag;
        }
    }

    /// Keeps only the flags at `indices`.
    pub(super) fn truncate_to(&mut self, indices: &[usize]) {
        self.0.retain(|(i, _)| indices.contains(i));
    }
}

/// A double-precision slot: the pieces of its doubleword and its flags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DoubleOut {
    /// `(name, mask within the doubleword, value)`.
    pub(crate) pieces: Vec<(&'static str, u64, Known<u64>)>,
    pub(crate) flags: DoubleFlags,
}

impl DoubleOut {
    fn value(bits: u64, flags: DoubleFlags) -> Self {
        Self {
            pieces: vec![("", u64::MAX, Known::Value(bits))],
            flags,
        }
    }
}

pub(super) fn double_slot_effect(
    effect: &mut Effect,
    insn: Insn,
    slot: usize,
    out: DoubleOut,
    flag_citation: &'static str,
) {
    let shift = 64 - 64 * slot;
    for (name, mask, value) in out.pieces {
        let piece = if name.is_empty() {
            format!("d{slot}")
        } else {
            format!("d{slot}.{name}")
        };
        let value = match value {
            Known::Value(bits) => Known::Value(u128::from(bits) << shift),
            Known::Open(q) => Known::Open(q),
        };
        push_write(effect, insn, u128::from(mask) << shift, piece, value);
    }
    let first = FPSCR_DOUBLE_FIRST[slot];
    for (index, raised) in out.flags.0 {
        effect.flags.push(FlagWrite {
            name: format!("d{slot}.{}", DoubleFlags::NAMES[index]),
            bit: first + index as u32,
            raised,
            citation: flag_citation,
        });
    }
}

/// The class of an IEEE value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IeeeClass {
    Zero,
    Denormal,
    Normal,
    Infinity,
    Quiet,
    Signaling,
}

/// The class and exact value of IEEE `bits` with `fraction_bits`
/// fraction bits and `exponent_bits` exponent bits.
///
/// [SPU-ISA p:197 s:9.2] an all-ones exponent with a nonzero fraction is a NaN, quiet when the fraction's high bit is 1.
fn ieee_operand(bits: u64, fraction_bits: u32, exponent_bits: u32) -> (IeeeClass, Dyadic) {
    let fraction = bits & ((1 << fraction_bits) - 1);
    let exponent = bits >> fraction_bits & ((1 << exponent_bits) - 1);
    let negative = bits >> (fraction_bits + exponent_bits) & 1 == 1;
    let all_ones = (1 << exponent_bits) - 1;
    let bias = i64::from((1u32 << (exponent_bits - 1)) - 1);
    let min_power = 1 - bias - i64::from(fraction_bits);
    match (exponent, fraction) {
        (0, 0) => (IeeeClass::Zero, Dyadic::zero(negative)),
        (0, f) => (
            IeeeClass::Denormal,
            Dyadic::new(negative, u128::from(f), min_power),
        ),
        (e, 0) if e == all_ones => (IeeeClass::Infinity, Dyadic::zero(negative)),
        (e, f) if e == all_ones => {
            let quiet = f >> (fraction_bits - 1) & 1 == 1;
            let class = if quiet {
                IeeeClass::Quiet
            } else {
                IeeeClass::Signaling
            };
            (class, Dyadic::zero(negative))
        }
        (e, f) => (
            IeeeClass::Normal,
            Dyadic::new(
                negative,
                u128::from(f | 1 << fraction_bits),
                e as i64 + min_power - 1,
            ),
        ),
    }
}

pub(crate) fn double_operand(bits: u64) -> (IeeeClass, Dyadic) {
    ieee_operand(bits, 52, 11)
}

pub(crate) fn single_ieee_operand(bits: u32) -> (IeeeClass, Dyadic) {
    ieee_operand(u64::from(bits), 23, 8)
}

const DEFAULT_QNAN: u64 = SPU_DEFAULT_QNAN_F64;
const DOUBLE_SIGN: u64 = 1 << 63;

/// A finite, infinite or invalid intermediate of the double arithmetic.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    Finite(Dyadic),
    Infinite(bool),
    Invalid,
}

/// IEEE addition of two non-NaN values, with the zero-sign rule.
///
/// [SPU-ISA p:197 s:9.2] normal IEEE semantics apply; [SPU-ISA p:199 s:9.2.2] magnitude subtraction of infinities is invalid. IEEE 754 gives an exact zero sum of opposite signs +0, or -0 rounding toward -infinity, and keeps the sign of a sum of two like zeros.
fn ieee_sum(x: Value, y: Value, mode: Mode) -> Value {
    match (x, y) {
        (Value::Invalid, _) | (_, Value::Invalid) => Value::Invalid,
        (Value::Infinite(a), Value::Infinite(b)) if a != b => Value::Invalid,
        (Value::Infinite(a), _) | (_, Value::Infinite(a)) => Value::Infinite(a),
        (Value::Finite(a), Value::Finite(b)) => {
            let sum = a.add(&b);
            if sum.is_zero() {
                let negative = if a.is_zero() && b.is_zero() && a.negative == b.negative {
                    a.negative
                } else {
                    mode == Mode::Down
                };
                Value::Finite(Dyadic::zero(negative))
            } else {
                Value::Finite(sum)
            }
        }
    }
}

/// IEEE multiplication of two non-NaN values.
///
/// [SPU-ISA p:199 s:9.2.2] infinity times zero is invalid.
fn ieee_product(x: &Value, y: &Value) -> Value {
    match (x, y) {
        (Value::Invalid, _) | (_, Value::Invalid) => Value::Invalid,
        (Value::Infinite(_), Value::Finite(z)) | (Value::Finite(z), Value::Infinite(_))
            if z.is_zero() =>
        {
            Value::Invalid
        }
        (Value::Infinite(a), Value::Infinite(b)) => Value::Infinite(a != b),
        (Value::Infinite(a), Value::Finite(z)) | (Value::Finite(z), Value::Infinite(a)) => {
            Value::Infinite(*a != z.negative)
        }
        (Value::Finite(a), Value::Finite(b)) => Value::Finite(a.mul(b)),
    }
}

fn negated(value: Value) -> Value {
    match value {
        Value::Finite(d) => Value::Finite(d.negated()),
        Value::Infinite(n) => Value::Infinite(!n),
        Value::Invalid => Value::Invalid,
    }
}

/// One slot of a double arithmetic instruction on `[ra, rb, rt]`.
///
/// [SPU-ISA p:203 s:9] dfa; [SPU-ISA p:205 s:9] dfs; [SPU-ISA p:207 s:9] dfm; [SPU-ISA p:209 s:9] dfma RA x RB + RT; [SPU-ISA p:213 s:9] dfms RA x RB - RT; [SPU-ISA p:211 s:9] dfnms and [SPU-ISA p:214 s:9] dfnma negate the rounded result unless it is a NaN, and a QNaN result has sign 0.
pub(crate) fn double_slot(op: Op, operands: [u64; 3], mode: Mode) -> DoubleOut {
    let used = match op {
        Op::Dfa | Op::Dfs | Op::Dfm => 2,
        _ => 3,
    };
    let decoded: Vec<(IeeeClass, Dyadic)> = operands[..used]
        .iter()
        .map(|&b| double_operand(b))
        .collect();
    let class = |i: usize| decoded[i].0;
    let any = |c: IeeeClass| (0..used).any(|i| class(i) == c);
    let negate = matches!(op, Op::Dfnms | Op::Dfnma);
    if any(IeeeClass::Quiet) || any(IeeeClass::Signaling) {
        // [SPU-ISA p:197 s:9.2] a NaN input gives a QNaN: the default or an input NaN, the NaN flag marking the first.
        let known_mask = if negate {
            DOUBLE_SIGN | DEFAULT_QNAN
        } else {
            DEFAULT_QNAN
        };
        let fused_infinity_zero = used == 3 && {
            let (p, q) = (class(0), class(1));
            let zero_like = |c| matches!(c, IeeeClass::Zero | IeeeClass::Denormal);
            (p == IeeeClass::Infinity && zero_like(q)) || (q == IeeeClass::Infinity && zero_like(p))
        };
        let invalid = if any(IeeeClass::Signaling) {
            SET
        } else if fused_infinity_zero {
            if class(0) == IeeeClass::Denormal || class(1) == IeeeClass::Denormal {
                Known::Open(Question::DoubleDenormalOperand)
            } else {
                Known::Open(Question::FusedInfinityZeroQnan)
            }
        } else if any(IeeeClass::Denormal) {
            // [SPU-ISA p:199 s:9.2.2] an implementation that flushes a denormal input may set INV beside DENORM.
            Known::Open(Question::DoubleDenormalOperand)
        } else {
            CLEAR
        };
        let denormal = if any(IeeeClass::Denormal) {
            Known::Open(Question::DoubleDenormalOperand)
        } else {
            CLEAR
        };
        return DoubleOut {
            pieces: vec![
                ("nan", known_mask, Known::Value(DEFAULT_QNAN)),
                (
                    "payload",
                    !known_mask,
                    Known::Open(Question::DoubleNanResult),
                ),
            ],
            flags: DoubleFlags::new([
                CLEAR,
                CLEAR,
                CLEAR,
                invalid,
                Known::Open(Question::DoubleNanResult),
                denormal,
            ]),
        };
    }
    if any(IeeeClass::Denormal) {
        let open = Known::Open(Question::DoubleDenormalOperand);
        let mut flags = DoubleFlags::uniform(open);
        flags.set(4, CLEAR);
        return DoubleOut {
            pieces: vec![("", u64::MAX, Known::Open(Question::DoubleDenormalOperand))],
            flags,
        };
    }
    let value = |i: usize| match decoded[i].0 {
        IeeeClass::Infinity => Value::Infinite(decoded[i].1.negative),
        _ => Value::Finite(decoded[i].1.clone()),
    };
    let result = match op {
        Op::Dfa => ieee_sum(value(0), value(1), mode),
        Op::Dfs => ieee_sum(value(0), negated(value(1)), mode),
        Op::Dfm => ieee_product(&value(0), &value(1)),
        Op::Dfma | Op::Dfnma => ieee_sum(ieee_product(&value(0), &value(1)), value(2), mode),
        _ => ieee_sum(ieee_product(&value(0), &value(1)), negated(value(2)), mode),
    };
    let (bits, flags) = match result {
        // [SPU-ISA p:197 s:9.2] with no NaN input a QNaN result is the default QNaN.
        Value::Invalid => (DEFAULT_QNAN, [CLEAR, CLEAR, CLEAR, SET, CLEAR, CLEAR]),
        Value::Infinite(negative) => (
            if negative { DOUBLE_SIGN } else { 0 } | DOUBLE.infinity(),
            [CLEAR; 6],
        ),
        Value::Finite(d) if d.is_zero() => (if d.negative { DOUBLE_SIGN } else { 0 }, [CLEAR; 6]),
        Value::Finite(d) => {
            let r = round(&d, DOUBLE, mode);
            (
                r.bits,
                [
                    Known::Value(r.overflow),
                    Known::Value(r.underflow),
                    Known::Value(r.inexact),
                    CLEAR,
                    CLEAR,
                    CLEAR,
                ],
            )
        }
    };
    let bits = if negate && bits != DEFAULT_QNAN {
        bits ^ DOUBLE_SIGN
    } else {
        bits
    };
    DoubleOut::value(bits, DoubleFlags::new(flags))
}

/// frds of one doubleword.
///
/// [SPU-ISA p:224 s:9] the double rounds to single in the slot's mode, into the left word; the right word is zero. [SPU-ISA p:198 s:9.2.1] the conversion is IEEE apart from denormal inputs, and single denormal results are produced.
pub(crate) fn round_to_single(bits: u64, mode: Mode) -> DoubleOut {
    let (class, value) = double_operand(bits);
    let left = |known: Known<u64>| match known {
        Known::Value(word) => Known::Value(word << 32),
        Known::Open(q) => Known::Open(q),
    };
    let right = ("right", 0xFFFF_FFFF, Known::Value(0));
    let sign = u64::from(bits >> 63 == 1) << 31;
    match class {
        IeeeClass::Quiet | IeeeClass::Signaling => DoubleOut {
            pieces: vec![
                ("nan", 0x7FC0_0000 << 32, Known::Value(0x7FC0_0000 << 32)),
                (
                    "payload",
                    0xFFFF_FFFF << 32 & !(0x7FC0_0000 << 32),
                    Known::Open(Question::DoubleNanResult),
                ),
                right,
            ],
            flags: DoubleFlags::new([
                CLEAR,
                CLEAR,
                CLEAR,
                Known::Value(class == IeeeClass::Signaling),
                Known::Open(Question::DoubleNanResult),
                CLEAR,
            ]),
        },
        IeeeClass::Denormal => {
            let open = Known::Open(Question::ConversionDenormalInput);
            DoubleOut {
                pieces: vec![
                    (
                        "left",
                        0xFFFF_FFFF << 32,
                        Known::Open(Question::ConversionDenormalInput),
                    ),
                    right,
                ],
                // [SPU-ISA p:199 s:9.2.2] an implementation that flushes a denormal input may set INV beside DENORM.
                flags: DoubleFlags::new([open, open, open, open, CLEAR, open]),
            }
        }
        IeeeClass::Infinity | IeeeClass::Zero => {
            let magnitude = if class == IeeeClass::Infinity {
                SINGLE.infinity()
            } else {
                0
            };
            DoubleOut {
                pieces: vec![
                    (
                        "left",
                        0xFFFF_FFFF << 32,
                        left(Known::Value(sign | magnitude)),
                    ),
                    right,
                ],
                flags: DoubleFlags::uniform(CLEAR),
            }
        }
        IeeeClass::Normal => {
            let r = round(&value, SINGLE, mode);
            DoubleOut {
                pieces: vec![
                    ("left", 0xFFFF_FFFF << 32, left(Known::Value(r.bits))),
                    right,
                ],
                flags: DoubleFlags::new([
                    Known::Value(r.overflow),
                    Known::Value(r.underflow),
                    Known::Value(r.inexact),
                    CLEAR,
                    CLEAR,
                    CLEAR,
                ]),
            }
        }
    }
}

/// fesd of the left word of one doubleword.
///
/// [SPU-ISA p:225 s:9] the single in the left word becomes a double; the right word is ignored. [SPU-ISA p:198 s:9.2.1] the conversion is IEEE apart from denormal inputs.
pub(crate) fn extend_to_double(word: u32) -> DoubleOut {
    let (class, value) = single_ieee_operand(word);
    let sign = u64::from(word >> 31) << 63;
    match class {
        IeeeClass::Quiet | IeeeClass::Signaling => DoubleOut {
            pieces: vec![
                ("nan", DEFAULT_QNAN, Known::Value(DEFAULT_QNAN)),
                (
                    "payload",
                    !DEFAULT_QNAN,
                    Known::Open(Question::DoubleNanResult),
                ),
            ],
            flags: DoubleFlags::new([
                CLEAR,
                CLEAR,
                CLEAR,
                Known::Value(class == IeeeClass::Signaling),
                Known::Open(Question::DoubleNanResult),
                CLEAR,
            ]),
        },
        IeeeClass::Denormal => {
            let open = Known::Open(Question::ConversionDenormalInput);
            DoubleOut {
                pieces: vec![("", u64::MAX, Known::Open(Question::ConversionDenormalInput))],
                // [SPU-ISA p:199 s:9.2.2] an implementation that flushes a denormal input may set INV beside DENORM.
                flags: DoubleFlags::new([CLEAR, CLEAR, CLEAR, open, CLEAR, open]),
            }
        }
        IeeeClass::Infinity => {
            DoubleOut::value(sign | DOUBLE.infinity(), DoubleFlags::uniform(CLEAR))
        }
        IeeeClass::Zero => DoubleOut::value(sign, DoubleFlags::uniform(CLEAR)),
        IeeeClass::Normal => {
            let r = round(&value, DOUBLE, Mode::Nearest);
            debug_assert!(!r.inexact);
            DoubleOut::value(r.bits, DoubleFlags::uniform(CLEAR))
        }
    }
}
