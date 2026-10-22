//! The single-precision families: the estimates, interpolate, and the
//! arithmetic forms.

use super::builder::{single_insn, Builder, SplitMix, RA, RB, RT};
use super::classes::{signed, single_cell, SINGLE_CLASSES};
use crate::spu_documented::model::{extended_operand, extended_result, Insn, Known, Op};
use crate::spu_documented::question::VectorClass;

/// [SPU-ISA p:215 s:9] the base fraction's top five bits index the estimate; [SPU-ISA p:217 s:9] frsqest also reads the exponent's parity.
pub(super) fn estimate(b: &mut Builder) {
    const EXPONENTS: [u64; 10] = [1, 2, 64, 126, 127, 128, 129, 200, 253, 254];
    for op in [Op::Frest, Op::Frsqest] {
        let insn = Insn::new(op, RT, RA, 0);
        let mut index_slots = Vec::new();
        for index in 0..32u32 {
            for &exponent in &EXPONENTS {
                for low in 0..3 {
                    let low_bits = match low {
                        0 => 0,
                        1 => 0x3_FFFF,
                        _ => b.rng.below(1 << 18) as u32,
                    };
                    let sign = u32::from(b.rng.bit()) << 31;
                    let word = sign | (exponent as u32) << 23 | index << 18 | low_bits;
                    index_slots.push((format!("{}:index{index:02}", op.mnemonic()), [word, 0, 0]));
                }
            }
        }
        b.single_vectors("index", VectorClass::Estimate, insn, index_slots);
        let mut exponent_slots = Vec::new();
        for exponent in 0..=255u32 {
            for fraction in [0, 0x7F_FFFF] {
                let sign = u32::from(b.rng.bit()) << 31;
                let word = sign | exponent << 23 | fraction;
                exponent_slots.push((
                    format!("{}:exponent{exponent:03}", op.mnemonic()),
                    [word, 0, 0],
                ));
            }
        }
        b.single_vectors("exponent", VectorClass::Estimate, insn, exponent_slots);
    }
}

/// [SPU-ISA p:219 s:9] fi reads RB as a base and a step and `RA[13:31]` as the fraction Y.
pub(super) fn interpolate(b: &mut Builder) {
    let insn = Insn::new(Op::Fi, RT, RA, RB);
    let mut edge = Vec::new();
    for exponent in [0u32, 1, 2, 127, 254, 255] {
        for base in [0u32, 0x1FFF] {
            for step in [0u32, 0x3FF] {
                for y in [0u32, 0x7_FFFF] {
                    let rb = u32::from(b.rng.bit()) << 31 | exponent << 23 | base << 10 | step;
                    let ra = 127 << 23 | (b.rng.below(16) as u32) << 19 | y;
                    edge.push((format!("fi:edge-e{exponent:03}"), [ra, rb, 0]));
                }
            }
        }
    }
    b.single_vectors("edge", VectorClass::Interpolate, insn, edge);
    let mut random = Vec::new();
    for _ in 0..448 {
        let rb_exponent = if b.rng.below(32) == 0 {
            0
        } else {
            b.rng.range(1, 255) as u32
        };
        let rb = u32::from(b.rng.bit()) << 31 | rb_exponent << 23 | b.rng.below(1 << 23) as u32;
        let (cell, ra_exponent) = match b.rng.below(16) {
            0 => ("fi:ra-exponent-zero", 0),
            1 => ("fi:ra-exponent-255", 255),
            _ => ("fi:random", b.rng.range(1, 254) as u32),
        };
        let ra =
            u32::from(b.rng.bit()) << 31 | ra_exponent << 23 | b.rng.range(1, (1 << 23) - 1) as u32;
        random.push((cell.to_owned(), [ra, rb, 0]));
    }
    b.single_vectors("random", VectorClass::Interpolate, insn, random);
}

/// A pair of extended-range operands whose exact product lies in
/// (Smax, 2^129).
fn band_product(rng: &mut SplitMix) -> (u32, u32) {
    // A 47-bit significand product above (2^24 - 1) x 2^23: its top 24
    // bits are all ones and a lower bit is set. With a just above 2^23
    // and c = floor((2^47 - 1) / a), the product is 2^47 - 1 less the
    // remainder, which is below 2^23 - 1 for most a.
    let threshold = ((1u64 << 24) - 1) << 23;
    let mut pair = (1 << 23, (1 << 24) - 1);
    for _ in 0..64 {
        let a = rng.range(1 << 23, (1 << 23) + (1 << 20));
        let c = ((1 << 47) - 1) / a;
        pair = (a, c);
        if a * c > threshold {
            break;
        }
    }
    // Smax is (2^24 - 1) x 2^105, so the 47-bit product needs the scale
    // 2^82: leading exponents 64 and 64, less 46 for the two 23-bit
    // fractions.
    let encode = |m: u64| (127 + 64) << 23 | (m as u32 & 0x7F_FFFF);
    (encode(pair.0), encode(pair.1))
}

/// [SPU-ISA p:196 s:9.1] the overflow, underflow and DIFF conditions; [SPU-ISA p:208 s:9] the fused product is exact.
pub(super) fn single_arithmetic(b: &mut Builder) {
    let classes = signed(&SINGLE_CLASSES);
    for op in [Op::Fa, Op::Fs, Op::Fm] {
        let mut slots = Vec::new();
        for &x in &classes {
            for &y in &classes {
                let words = [x, y].map(|(c, s)| c.draw(&mut b.rng) | u32::from(s) << 31);
                slots.push((single_cell(op, &[x, y]), [words[0], words[1], 0]));
            }
        }
        b.single_vectors(
            "cell",
            VectorClass::SingleArithmetic,
            single_insn(op),
            slots,
        );
    }
    for op in [Op::Fma, Op::Fms, Op::Fnms] {
        let mut slots = Vec::new();
        for &x in &classes {
            for &y in &classes {
                for &z in &classes {
                    let words = [x, y, z].map(|(c, s)| c.draw(&mut b.rng) | u32::from(s) << 31);
                    slots.push((single_cell(op, &[x, y, z]), words));
                }
            }
        }
        b.single_vectors(
            "cell",
            VectorClass::SingleArithmetic,
            single_insn(op),
            slots,
        );
    }
    for op in [Op::Fa, Op::Fs, Op::Fm, Op::Fma, Op::Fms, Op::Fnms] {
        let mut slots = Vec::new();
        for n in 0..16 {
            let negative = b.rng.bit();
            let sign = u32::from(negative) << 31;
            // Below 2^104, half the spacing of the numbers next to Smax.
            let small = sign | (b.rng.range(200, 230) as u32) << 23 | b.rng.below(1 << 23) as u32;
            // A product near 2^46, far below 2^104.
            let tiny = 150 << 23 | b.rng.below(1 << 23) as u32;
            let (p, q) = band_product(&mut b.rng);
            let smax = sign | 0x7FFF_FFFF;
            let words = match (op, n % 2) {
                (Op::Fa, _) => [smax, small, 0],
                (Op::Fs, _) => [smax, small ^ 0x8000_0000, 0],
                (Op::Fm, _) => [p | sign, q, 0],
                // fma: RA x RB + RC.
                (Op::Fma, 0) => [p | sign, q, 0],
                (Op::Fma, _) => [tiny | sign, tiny, smax],
                // fms: RA x RB - RC.
                (Op::Fms, 0) => [p | sign, q, 0],
                (Op::Fms, _) => [tiny | sign, tiny, smax ^ 0x8000_0000],
                // fnms: RC - RA x RB.
                (_, 0) => [p | sign, q, 0],
                (_, _) => [tiny | sign, tiny, smax ^ 0x8000_0000],
            };
            slots.push((format!("{}:overflow-band", op.mnemonic()), words));
        }
        b.single_vectors(
            "band",
            VectorClass::SingleArithmetic,
            single_insn(op),
            slots,
        );
    }
    for op in [Op::Fma, Op::Fms, Op::Fnms] {
        let mut slots = Vec::new();
        for difference in -60i64..=60 {
            let operand = |exponent: i64, rng: &mut SplitMix| {
                u32::from(rng.bit()) << 31 | (exponent as u32) << 23 | rng.below(1 << 23) as u32
            };
            let words = [
                operand(127, &mut b.rng),
                operand(127, &mut b.rng),
                operand(127 - difference, &mut b.rng),
            ];
            slots.push((
                format!("{}:exponent-difference{difference:+03}", op.mnemonic()),
                words,
            ));
        }
        b.single_vectors(
            "difference",
            VectorClass::SingleArithmetic,
            single_insn(op),
            slots,
        );
    }
    for op in [Op::Fa, Op::Fs, Op::Fma, Op::Fms, Op::Fnms] {
        let mut slots = Vec::new();
        for _ in 0..8 {
            // Twelve-bit significands keep the product exact in 24 bits.
            let short = |rng: &mut SplitMix| {
                u32::from(rng.bit()) << 31
                    | (rng.range(100, 154) as u32) << 23
                    | (rng.below(1 << 11) as u32) << 12
            };
            let (x, y) = (short(&mut b.rng), short(&mut b.rng));
            let product =
                match extended_result(&extended_operand(x).0.mul(&extended_operand(y).0), false)
                    .bits
                {
                    Known::Value(bits) => bits,
                    Known::Open(_) => 0,
                };
            let words = match op {
                Op::Fa => [x, x ^ 0x8000_0000, 0],
                Op::Fs => [x, x, 0],
                Op::Fma => [x, y, product ^ 0x8000_0000],
                _ => [x, y, product],
            };
            slots.push((format!("{}:exact-zero", op.mnemonic()), words));
        }
        b.single_vectors(
            "exact-zero",
            VectorClass::SingleArithmetic,
            single_insn(op),
            slots,
        );
    }
    for op in [Op::Fa, Op::Fs, Op::Fm, Op::Fma, Op::Fms, Op::Fnms] {
        let slots = (0..100)
            .map(|_| {
                let words = [
                    b.rng.next() as u32,
                    b.rng.next() as u32,
                    b.rng.next() as u32,
                ];
                (format!("{}:random", op.mnemonic()), words)
            })
            .collect();
        b.single_vectors(
            "random",
            VectorClass::SingleArithmetic,
            single_insn(op),
            slots,
        );
    }
}
