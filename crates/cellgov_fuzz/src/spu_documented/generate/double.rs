//! The double-precision families: the arithmetic forms, and frds and
//! fesd.

use super::builder::{Builder, SplitMix, RA, RB, RT};
use super::classes::{double_cell, mode_name, signed, signed_name, DOUBLE_CLASSES, FUSED_CLASSES};
use crate::spu_documented::dyadic::{self, Mode};
use crate::spu_documented::model::{self, Insn, Op};
use crate::spu_documented::question::VectorClass;

/// A double operand with its exponent field in `[low, high]`.
fn double_in(rng: &mut SplitMix, negative: bool, low: u64, high: u64) -> u64 {
    u64::from(negative) << 63 | rng.range(low, high) << 52 | rng.below(1 << 52)
}

/// [SPU-ISA p:197 s:9.2] normal IEEE semantics in four rounding modes; [SPU-ISA p:199 s:9.2.2] UNF is tiny before rounding and inexact.
pub(super) fn double_arithmetic(b: &mut Builder) {
    let classes = signed(&DOUBLE_CLASSES);
    for op in [Op::Dfa, Op::Dfs, Op::Dfm] {
        let mut slots = Vec::new();
        for &x in &classes {
            for &y in &classes {
                for mode in Mode::ALL {
                    let words = [x, y].map(|(c, s)| c.draw(&mut b.rng) | u64::from(s) << 63);
                    slots.push((
                        double_cell(op, &[x, y], mode),
                        [words[0], words[1], 0],
                        mode,
                    ));
                }
            }
        }
        b.double_vectors(
            "cell",
            VectorClass::DoubleArithmetic,
            Insn::new(op, RT, RA, RB),
            slots,
        );
    }
    let fused = signed(&FUSED_CLASSES);
    for op in [Op::Dfma, Op::Dfms, Op::Dfnms, Op::Dfnma] {
        let mut slots = Vec::new();
        let mut n = 0;
        for &x in &fused {
            for &y in &fused {
                for &z in &fused {
                    let mode = Mode::ALL[n % 4];
                    n += 1;
                    let words = [x, y, z].map(|(c, s)| c.draw(&mut b.rng) | u64::from(s) << 63);
                    slots.push((double_cell(op, &[x, y, z], mode), words, mode));
                }
            }
        }
        b.double_vectors(
            "cell",
            VectorClass::DoubleArithmetic,
            Insn::new(op, RT, RA, RB),
            slots,
        );
    }
    for op in [Op::Dfa, Op::Dfs, Op::Dfm, Op::Dfma, Op::Dfnms] {
        let mut slots = Vec::new();
        for mode in Mode::ALL {
            for _ in 0..16 {
                let (s0, s1, s2) = (b.rng.bit(), b.rng.bit(), b.rng.bit());
                let words = match op {
                    // Products near 2^-1022 from both sides.
                    Op::Dfm => [
                        double_in(&mut b.rng, s0, 508, 515),
                        double_in(&mut b.rng, s1, 508, 515),
                        0,
                    ],
                    // Sums of two numbers near the smallest normal: exact
                    // denormal differences and normal sums.
                    Op::Dfa | Op::Dfs => [
                        double_in(&mut b.rng, s0, 1, 3),
                        double_in(&mut b.rng, s1, 1, 3),
                        0,
                    ],
                    _ => [
                        double_in(&mut b.rng, s0, 508, 515),
                        double_in(&mut b.rng, s1, 508, 515),
                        double_in(&mut b.rng, s2, 1, 3),
                    ],
                };
                slots.push((
                    format!("{}:underflow@{}", op.mnemonic(), mode_name(mode)),
                    words,
                    mode,
                ));
            }
        }
        b.double_vectors(
            "underflow",
            VectorClass::DoubleArithmetic,
            Insn::new(op, RT, RA, RB),
            slots,
        );
    }
    for op in [Op::Dfa, Op::Dfs, Op::Dfma, Op::Dfms, Op::Dfnms, Op::Dfnma] {
        let mut slots = Vec::new();
        for mode in Mode::ALL {
            for _ in 0..4 {
                // 26-bit significands keep the product exact in 53 bits.
                let short = |rng: &mut SplitMix| {
                    u64::from(rng.bit()) << 63
                        | rng.range(990, 1060) << 52
                        | rng.below(1 << 25) << 27
                };
                let (x, y) = (short(&mut b.rng), short(&mut b.rng));
                let product = exact_double_product(x, y);
                let words = match op {
                    Op::Dfa => [x, x ^ 1 << 63, 0],
                    Op::Dfs => [x, x, 0],
                    Op::Dfma | Op::Dfnma => [x, y, product ^ 1 << 63],
                    _ => [x, y, product],
                };
                slots.push((
                    format!("{}:exact-zero@{}", op.mnemonic(), mode_name(mode)),
                    words,
                    mode,
                ));
            }
        }
        b.double_vectors(
            "exact-zero",
            VectorClass::DoubleArithmetic,
            Insn::new(op, RT, RA, RB),
            slots,
        );
    }
}

/// The product of two doubles whose significands fit 26 bits, exactly.
fn exact_double_product(x: u64, y: u64) -> u64 {
    let (_, a) = model::double_operand(x);
    let (_, c) = model::double_operand(y);
    dyadic::round(&a.mul(&c), dyadic::DOUBLE, Mode::Nearest).bits
}

/// The double classes frds covers, by the single result they make.
pub(crate) const FRDS_CLASSES: [&str; 10] = [
    "zero",
    "denormal",
    "in-range",
    "overflow",
    "subnormal",
    "deep",
    "halfway",
    "inf",
    "qnan",
    "snan",
];

/// The single classes fesd covers.
pub(crate) const FESD_CLASSES: [&str; 7] = [
    "zero", "denormal", "normal", "exp254", "inf", "qnan", "snan",
];

/// [SPU-ISA p:224 s:9] frds rounds in the slot's mode; [SPU-ISA p:225 s:9] fesd converts the left word; [SPU-ISA p:198 s:9.2.1] both are IEEE apart from denormal inputs.
pub(super) fn round_extend(b: &mut Builder) {
    let mut slots = Vec::new();
    for class in FRDS_CLASSES {
        for negative in [false, true] {
            for mode in Mode::ALL {
                for _ in 0..3 {
                    let r = &mut b.rng;
                    let magnitude = match class {
                        "zero" => 0,
                        "denormal" => r.range(1, (1 << 52) - 1),
                        "in-range" => double_in(r, false, 1023 - 126, 1023 + 127),
                        "overflow" => double_in(r, false, 1023 + 128, 2046),
                        "subnormal" => double_in(r, false, 1023 - 149, 1023 - 127),
                        "deep" => double_in(r, false, 1023 - 300, 1023 - 151),
                        // Exactly half way between two singles.
                        "halfway" => {
                            r.range(1023 - 126, 1023 + 127) << 52 | r.below(1 << 23) << 29 | 1 << 28
                        }
                        "inf" => 0x7FF0_0000_0000_0000,
                        "qnan" => 0x7FF8_0000_0000_0000 | r.below(1 << 51),
                        _ => 0x7FF0_0000_0000_0000 | r.range(1, (1 << 51) - 1),
                    };
                    let bits = u64::from(negative) << 63 | magnitude;
                    let cell = format!("frds:{}@{}", signed_name(class, negative), mode_name(mode));
                    slots.push((cell, [bits, 0, 0], mode));
                }
            }
        }
    }
    b.double_vectors(
        "cell",
        VectorClass::RoundExtend,
        Insn::new(Op::Frds, RT, RA, 0),
        slots,
    );
    let mut slots = Vec::new();
    for class in FESD_CLASSES {
        for negative in [false, true] {
            for _ in 0..5 {
                let r = &mut b.rng;
                let magnitude = match class {
                    "zero" => 0,
                    "denormal" => r.range(1, (1 << 23) - 1) as u32,
                    "normal" => (r.range(1, 253) as u32) << 23 | r.below(1 << 23) as u32,
                    "exp254" => 254 << 23 | r.below(1 << 23) as u32,
                    "inf" => 0x7F80_0000,
                    "qnan" => 0x7FC0_0000 | r.below(1 << 22) as u32,
                    _ => 0x7F80_0000 | r.range(1, (1 << 22) - 1) as u32,
                };
                let word = u32::from(negative) << 31 | magnitude;
                // fesd ignores the right word, so it carries noise.
                let bits = u64::from(word) << 32 | r.next() >> 32;
                let mode = Mode::ALL[r.below(4) as usize];
                slots.push((
                    format!("fesd:{}", signed_name(class, negative)),
                    [bits, 0, 0],
                    mode,
                ));
            }
        }
    }
    b.double_vectors(
        "cell",
        VectorClass::RoundExtend,
        Insn::new(Op::Fesd, RT, RA, 0),
        slots,
    );
}
