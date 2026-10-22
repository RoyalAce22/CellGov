//! The operand classes the generator crosses, and the names of their
//! cells.

use super::builder::SplitMix;
use crate::spu_documented::dyadic::Mode;
use crate::spu_documented::model::Op;

/// The magnitude classes of an extended-range single-precision operand.
///
/// [SPU-ISA p:195 s:9.1] Table 9-1: Smin is x'00800000' and Smax x'7FFFFFFF'; exponent 255 is a normal binade.
/// [SPU-ISA p:196 s:9.1] a zero exponent with a nonzero fraction is a denorm, read as zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SingleClass {
    Zero,
    Denormal,
    Smin,
    Low,
    One,
    Mid,
    High,
    Exp254,
    Exp255,
    Smax,
}

pub(crate) const SINGLE_CLASSES: [SingleClass; 10] = [
    SingleClass::Zero,
    SingleClass::Denormal,
    SingleClass::Smin,
    SingleClass::Low,
    SingleClass::One,
    SingleClass::Mid,
    SingleClass::High,
    SingleClass::Exp254,
    SingleClass::Exp255,
    SingleClass::Smax,
];

impl SingleClass {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Zero => "zero",
            Self::Denormal => "denormal",
            Self::Smin => "smin",
            Self::Low => "low",
            Self::One => "one",
            Self::Mid => "mid",
            Self::High => "high",
            Self::Exp254 => "exp254",
            Self::Exp255 => "exp255",
            Self::Smax => "smax",
        }
    }

    pub(super) fn draw(self, rng: &mut SplitMix) -> u32 {
        let fraction = |rng: &mut SplitMix| rng.below(1 << 23) as u32;
        let with = |exponent: u64, rng: &mut SplitMix| (exponent as u32) << 23 | fraction(rng);
        match self {
            Self::Zero => 0,
            Self::Denormal => rng.range(1, (1 << 23) - 1) as u32,
            Self::Smin => 0x0080_0000,
            Self::Low => {
                let e = rng.range(2, 20);
                with(e, rng)
            }
            Self::One => 0x3F80_0000,
            Self::Mid => {
                let e = rng.range(100, 154);
                with(e, rng)
            }
            Self::High => {
                let e = rng.range(230, 253);
                with(e, rng)
            }
            Self::Exp254 => with(254, rng),
            Self::Exp255 => with(255, rng),
            Self::Smax => 0x7FFF_FFFF,
        }
    }
}

/// The magnitude classes of a double-precision operand.
///
/// [SPU-ISA p:197 s:9.2] Table 9-3: Dmin x'0000000000000001', Dmax x'7FEFFFFFFFFFFFFF'; an all-ones exponent is a NaN or an infinity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DoubleClass {
    Zero,
    Denormal,
    MinNormal,
    Small,
    Mid,
    Large,
    Max,
    Infinity,
    Quiet,
    Signaling,
}

pub(crate) const DOUBLE_CLASSES: [DoubleClass; 10] = [
    DoubleClass::Zero,
    DoubleClass::Denormal,
    DoubleClass::MinNormal,
    DoubleClass::Small,
    DoubleClass::Mid,
    DoubleClass::Large,
    DoubleClass::Max,
    DoubleClass::Infinity,
    DoubleClass::Quiet,
    DoubleClass::Signaling,
];

/// The classes the three-operand forms cross.
pub(crate) const FUSED_CLASSES: [DoubleClass; 7] = [
    DoubleClass::Zero,
    DoubleClass::Denormal,
    DoubleClass::Mid,
    DoubleClass::Large,
    DoubleClass::Infinity,
    DoubleClass::Quiet,
    DoubleClass::Signaling,
];

impl DoubleClass {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Zero => "zero",
            Self::Denormal => "denormal",
            Self::MinNormal => "min-normal",
            Self::Small => "small",
            Self::Mid => "mid",
            Self::Large => "large",
            Self::Max => "max",
            Self::Infinity => "inf",
            Self::Quiet => "qnan",
            Self::Signaling => "snan",
        }
    }

    pub(super) fn draw(self, rng: &mut SplitMix) -> u64 {
        let fraction = |rng: &mut SplitMix| rng.below(1 << 52);
        let with = |exponent: u64, rng: &mut SplitMix| exponent << 52 | fraction(rng);
        match self {
            Self::Zero => 0,
            Self::Denormal => rng.range(1, (1 << 52) - 1),
            Self::MinNormal => 1 << 52,
            Self::Small => {
                let e = rng.range(2, 60);
                with(e, rng)
            }
            Self::Mid => {
                let e = rng.range(990, 1060);
                with(e, rng)
            }
            Self::Large => {
                let e = rng.range(1990, 2045);
                with(e, rng)
            }
            Self::Max => 0x7FEF_FFFF_FFFF_FFFF,
            Self::Infinity => 0x7FF0_0000_0000_0000,
            Self::Quiet => 0x7FF8_0000_0000_0000 | rng.below(1 << 51),
            Self::Signaling => 0x7FF0_0000_0000_0000 | rng.range(1, (1 << 51) - 1),
        }
    }
}

pub(crate) fn signed_name(name: &str, negative: bool) -> String {
    format!("{}{name}", if negative { '-' } else { '+' })
}

pub(crate) fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Nearest => "rn",
        Mode::Zero => "rz",
        Mode::Up => "rp",
        Mode::Down => "rm",
    }
}

/// The cell name of one single-precision slot.
pub(crate) fn single_cell(op: Op, classes: &[(SingleClass, bool)]) -> String {
    let names: Vec<String> = classes
        .iter()
        .map(|&(c, s)| signed_name(c.name(), s))
        .collect();
    format!("{}:{}", op.mnemonic(), names.join(","))
}

/// The cell name of one double-precision slot.
pub(crate) fn double_cell(op: Op, classes: &[(DoubleClass, bool)], mode: Mode) -> String {
    let names: Vec<String> = classes
        .iter()
        .map(|&(c, s)| signed_name(c.name(), s))
        .collect();
    format!("{}:{}@{}", op.mnemonic(), names.join(","), mode_name(mode))
}

/// Every signed class of `classes`, in order.
pub(super) fn signed<T: Copy>(classes: &[T]) -> Vec<(T, bool)> {
    classes
        .iter()
        .flat_map(|&c| [(c, false), (c, true)])
        .collect()
}
