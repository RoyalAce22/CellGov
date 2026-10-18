//! Exact binary values for the document model: an unbounded integer
//! significand times a power of two, and the rounding of such a value
//! into a binary format.
//!
//! The model shares no code with `cellgov_float`. Its values come from
//! the documents' definitions, computed with unbounded integers, so a
//! defect in the engine cannot also hide in the expected value.

use std::cmp::Ordering;

/// An unsigned integer of any width, least significant limb first, with
/// no high zero limb.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Natural(Vec<u64>);

impl Natural {
    pub(crate) fn zero() -> Self {
        Self(Vec::new())
    }

    pub(crate) fn from_u128(value: u128) -> Self {
        let mut n = Self(vec![value as u64, (value >> 64) as u64]);
        n.trim();
        n
    }

    fn trim(&mut self) {
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
    }

    pub(crate) fn is_zero(&self) -> bool {
        self.0.is_empty()
    }

    /// The position of the highest set bit plus one; 0 for zero.
    pub(crate) fn bits(&self) -> u64 {
        match self.0.last() {
            None => 0,
            Some(&top) => (self.0.len() as u64 - 1) * 64 + u64::from(64 - top.leading_zeros()),
        }
    }

    pub(crate) fn bit(&self, index: u64) -> bool {
        let limb = (index / 64) as usize;
        limb < self.0.len() && self.0[limb] >> (index % 64) & 1 == 1
    }

    /// Whether any bit below `index` is set.
    pub(crate) fn any_below(&self, index: u64) -> bool {
        let whole = (index / 64) as usize;
        let part = index % 64;
        if self.0.iter().take(whole).any(|&limb| limb != 0) {
            return true;
        }
        part != 0 && whole < self.0.len() && self.0[whole] & ((1u64 << part) - 1) != 0
    }

    pub(crate) fn shl(&self, shift: u64) -> Self {
        if self.is_zero() {
            return Self::zero();
        }
        let whole = (shift / 64) as usize;
        let part = shift % 64;
        let mut limbs = vec![0u64; whole];
        let mut carry = 0u64;
        for &limb in &self.0 {
            if part == 0 {
                limbs.push(limb);
            } else {
                limbs.push(limb << part | carry);
                carry = limb >> (64 - part);
            }
        }
        limbs.push(carry);
        let mut n = Self(limbs);
        n.trim();
        n
    }

    pub(crate) fn shr(&self, shift: u64) -> Self {
        let whole = (shift / 64) as usize;
        if whole >= self.0.len() {
            return Self::zero();
        }
        let part = shift % 64;
        let rest = &self.0[whole..];
        let limbs = (0..rest.len())
            .map(|i| {
                let high = rest.get(i + 1).copied().unwrap_or(0);
                if part == 0 {
                    rest[i]
                } else {
                    rest[i] >> part | high << (64 - part)
                }
            })
            .collect();
        let mut n = Self(limbs);
        n.trim();
        n
    }

    pub(crate) fn add(&self, other: &Self) -> Self {
        let len = self.0.len().max(other.0.len());
        let mut limbs = Vec::with_capacity(len + 1);
        let mut carry = false;
        for i in 0..len {
            let a = self.0.get(i).copied().unwrap_or(0);
            let b = other.0.get(i).copied().unwrap_or(0);
            let (sum, c1) = a.overflowing_add(b);
            let (sum, c2) = sum.overflowing_add(u64::from(carry));
            limbs.push(sum);
            carry = c1 || c2;
        }
        limbs.push(u64::from(carry));
        let mut n = Self(limbs);
        n.trim();
        n
    }

    /// `self - other`, for `self >= other`.
    pub(crate) fn sub(&self, other: &Self) -> Self {
        debug_assert!(self.cmp(other) != Ordering::Less);
        let mut limbs = Vec::with_capacity(self.0.len());
        let mut borrow = false;
        for i in 0..self.0.len() {
            let b = other.0.get(i).copied().unwrap_or(0);
            let (diff, b1) = self.0[i].overflowing_sub(b);
            let (diff, b2) = diff.overflowing_sub(u64::from(borrow));
            limbs.push(diff);
            borrow = b1 || b2;
        }
        let mut n = Self(limbs);
        n.trim();
        n
    }

    pub(crate) fn mul(&self, other: &Self) -> Self {
        if self.is_zero() || other.is_zero() {
            return Self::zero();
        }
        let mut limbs = vec![0u64; self.0.len() + other.0.len()];
        for (i, &a) in self.0.iter().enumerate() {
            let mut carry = 0u128;
            for (j, &b) in other.0.iter().enumerate() {
                let t = u128::from(a) * u128::from(b) + u128::from(limbs[i + j]) + carry;
                limbs[i + j] = t as u64;
                carry = t >> 64;
            }
            limbs[i + other.0.len()] = carry as u64;
        }
        let mut n = Self(limbs);
        n.trim();
        n
    }

    pub(crate) fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .len()
            .cmp(&other.0.len())
            .then_with(|| self.0.iter().rev().cmp(other.0.iter().rev()))
    }

    pub(crate) fn to_u128(&self) -> Option<u128> {
        match self.0.as_slice() {
            [] => Some(0),
            [low] => Some(u128::from(*low)),
            [low, high] => Some(u128::from(*high) << 64 | u128::from(*low)),
            _ => None,
        }
    }
}

/// `(-1)^negative x magnitude x 2^exponent`. A zero keeps its sign; the
/// caller decides it by the rule of the operation that produced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Dyadic {
    pub(crate) negative: bool,
    pub(crate) magnitude: Natural,
    pub(crate) exponent: i64,
}

impl Dyadic {
    pub(crate) fn zero(negative: bool) -> Self {
        Self {
            negative,
            magnitude: Natural::zero(),
            exponent: 0,
        }
    }

    pub(crate) fn new(negative: bool, magnitude: u128, exponent: i64) -> Self {
        Self {
            negative,
            magnitude: Natural::from_u128(magnitude),
            exponent,
        }
    }

    pub(crate) fn is_zero(&self) -> bool {
        self.magnitude.is_zero()
    }

    pub(crate) fn negated(&self) -> Self {
        Self {
            negative: !self.negative,
            ..self.clone()
        }
    }

    /// The exponent of the highest set bit: the value lies in
    /// `[2^e, 2^(e+1))`. The result is meaningless for zero.
    pub(crate) fn leading_exponent(&self) -> i64 {
        self.magnitude.bits() as i64 - 1 + self.exponent
    }

    /// The exact sum. An exact zero sum is positive.
    pub(crate) fn add(&self, other: &Self) -> Self {
        if self.is_zero() {
            return other.clone();
        }
        if other.is_zero() {
            return self.clone();
        }
        let exponent = self.exponent.min(other.exponent);
        let a = self.magnitude.shl((self.exponent - exponent) as u64);
        let b = other.magnitude.shl((other.exponent - exponent) as u64);
        if self.negative == other.negative {
            return Self {
                negative: self.negative,
                magnitude: a.add(&b),
                exponent,
            };
        }
        match a.cmp(&b) {
            Ordering::Equal => Self::zero(false),
            Ordering::Greater => Self {
                negative: self.negative,
                magnitude: a.sub(&b),
                exponent,
            },
            Ordering::Less => Self {
                negative: other.negative,
                magnitude: b.sub(&a),
                exponent,
            },
        }
    }

    /// The exact product; its sign is the exclusive or of the signs.
    pub(crate) fn mul(&self, other: &Self) -> Self {
        Self {
            negative: self.negative != other.negative,
            magnitude: self.magnitude.mul(&other.magnitude),
            exponent: self.exponent + other.exponent,
        }
    }

    /// The value times `2^power`.
    pub(crate) fn scaled(&self, power: i64) -> Self {
        Self {
            exponent: self.exponent + power,
            ..self.clone()
        }
    }

    /// The magnitude truncated toward zero to an integer, and whether a
    /// fraction was dropped.
    pub(crate) fn truncated_magnitude(&self) -> (Natural, bool) {
        if self.exponent >= 0 {
            (self.magnitude.shl(self.exponent as u64), false)
        } else {
            let shift = self.exponent.unsigned_abs();
            (self.magnitude.shr(shift), self.magnitude.any_below(shift))
        }
    }
}

/// An IEEE rounding direction.
///
/// [SPU-ISA p:200 s:9.3] the RN field encodes 00 nearest even, 01 toward zero, 10 toward +infinity, 11 toward -infinity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Nearest,
    Zero,
    Up,
    Down,
}

impl Mode {
    pub(crate) const ALL: [Self; 4] = [Self::Nearest, Self::Zero, Self::Up, Self::Down];

    /// The mode a two-bit RN field names.
    pub(crate) fn from_field(field: u32) -> Self {
        Self::ALL[(field & 3) as usize]
    }

    pub(crate) fn field(self) -> u32 {
        match self {
            Self::Nearest => 0,
            Self::Zero => 1,
            Self::Up => 2,
            Self::Down => 3,
        }
    }
}

/// An IEEE binary interchange format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Format {
    /// Significand bits, the hidden bit included.
    pub(crate) precision: u64,
    /// The smallest normal exponent.
    pub(crate) min_exponent: i64,
    /// The largest finite exponent.
    pub(crate) max_exponent: i64,
}

impl Format {
    pub(crate) const fn bias(self) -> i64 {
        self.max_exponent
    }

    pub(crate) const fn exponent_all_ones(self) -> u64 {
        (self.max_exponent as u64 + 1) * 2 - 1
    }

    pub(crate) const fn fraction_bits(self) -> u64 {
        self.precision - 1
    }

    pub(crate) const fn sign_bit(self) -> u64 {
        1 << (self.fraction_bits() + self.exponent_bits())
    }

    const fn exponent_bits(self) -> u64 {
        64 - (self.exponent_all_ones()).leading_zeros() as u64
    }

    pub(crate) const fn infinity(self) -> u64 {
        self.exponent_all_ones() << self.fraction_bits()
    }

    pub(crate) const fn max_finite(self) -> u64 {
        self.infinity() - 1
    }
}

/// [SPU-ISA p:197 s:9.2] Table 9-3: 11-bit exponent biased by 1023, 52-bit fraction.
pub(crate) const DOUBLE: Format = Format {
    precision: 53,
    min_exponent: -1022,
    max_exponent: 1023,
};

/// [SPU-ISA p:198 s:9.2.1] Table 9-4: IEEE single precision, 8-bit exponent biased by 127, 23-bit fraction.
pub(crate) const SINGLE: Format = Format {
    precision: 24,
    min_exponent: -126,
    max_exponent: 127,
};

/// A value rounded into a format, with the IEEE exceptions it raised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rounded {
    pub(crate) bits: u64,
    pub(crate) inexact: bool,
    pub(crate) overflow: bool,
    pub(crate) underflow: bool,
}

/// Rounds a nonzero exact value into `format` in `mode`.
///
/// [SPU-ISA p:198 s:9.2.2] INX: the delivered value differs from the one an unbounded exponent range and precision give. OVF: the value rounded with an unbounded exponent range exceeds the largest finite number.
/// [SPU-ISA p:199 s:9.2.2] UNF is tiny before rounding and inexact; tiny is a nonzero result below the smallest normal number with the exponent range unbounded.
pub(crate) fn round(value: &Dyadic, format: Format, mode: Mode) -> Rounded {
    debug_assert!(!value.is_zero());
    let precision = format.precision as i64;
    let leading = value.leading_exponent();
    let tiny = leading < format.min_exponent;
    // The weight of the last kept bit.
    let mut quantum = leading.max(format.min_exponent) - (precision - 1);
    let shift = value.exponent - quantum;
    let (mut kept, half, rest) = if shift >= 0 {
        (value.magnitude.shl(shift as u64), false, false)
    } else {
        let s = shift.unsigned_abs();
        (
            value.magnitude.shr(s),
            value.magnitude.bit(s - 1),
            value.magnitude.any_below(s - 1),
        )
    };
    let inexact = half || rest;
    let negative = value.negative;
    let up = match mode {
        Mode::Nearest => half && (rest || kept.bit(0)),
        Mode::Zero => false,
        Mode::Up => inexact && !negative,
        Mode::Down => inexact && negative,
    };
    if up {
        kept = kept.add(&Natural::from_u128(1));
    }
    if kept.bits() as i64 > precision {
        kept = kept.shr(1);
        quantum += 1;
    }
    let kept = kept
        .to_u128()
        .expect("invariant: a rounded significand has at most 53 bits") as u64;
    let sign = if negative { format.sign_bit() } else { 0 };
    let normal = kept >> format.fraction_bits() == 1;
    let exponent = quantum + precision - 1;
    if normal && exponent > format.max_exponent {
        let to_infinity = match mode {
            Mode::Nearest => true,
            Mode::Zero => false,
            Mode::Up => !negative,
            Mode::Down => negative,
        };
        let magnitude = if to_infinity {
            format.infinity()
        } else {
            format.max_finite()
        };
        return Rounded {
            bits: sign | magnitude,
            inexact: true,
            overflow: true,
            underflow: false,
        };
    }
    let biased = if normal {
        (exponent + format.bias()) as u64
    } else {
        0
    };
    let fraction = kept & ((1 << format.fraction_bits()) - 1);
    Rounded {
        bits: sign | biased << format.fraction_bits() | fraction,
        inexact,
        overflow: false,
        underflow: tiny && inexact,
    }
}
