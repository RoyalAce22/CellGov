//! The deterministic generator of the documented floating-point list.
//!
//! The generator draws each operand from a class of the operand-class
//! model and emits every combination of classes per operand. A boundary
//! that random inputs rarely reach still gets a vector.
//!
//! [Aharoni2003 p:18 s:2.1] a floating-point test is a set of operands chosen from classes, with constraints on the intermediate result.
//! [Aharoni2003 p:21 s:4.2] a generator that solves for every task covers the whole model, where random inputs leave part of it unreached.

use std::collections::BTreeMap;

use cellgov_ps3_abi::hw::spu_fpscr::FPSCR_DEFINED;
use cellgov_ps3_abi::hw::spu_isa::{TO_FLOAT_SCALE_BIAS, TO_INTEGER_SCALE_BIAS};

use super::dyadic::Mode;
use super::model::{extended_operand, extended_result, with_mode, Insn, Known, Op};
use super::question::VectorClass;
use super::vector::{build, DocumentedVector};

/// SplitMix64: every host draws the same sequence.
struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `[0, n)`; `n` is nonzero.
    fn below(&mut self, n: u64) -> u64 {
        ((u128::from(self.next()) * u128::from(n)) >> 64) as u64
    }

    /// A value in `[low, high]`.
    fn range(&mut self, low: u64, high: u64) -> u64 {
        low + self.below(high - low + 1)
    }

    fn bit(&mut self) -> bool {
        self.next() >> 63 == 1
    }
}

/// The seed of the whole list.
const SEED: u64 = 0x0141_1D0C_F10A_7000;

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

    fn draw(self, rng: &mut SplitMix) -> u32 {
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

    fn draw(self, rng: &mut SplitMix) -> u64 {
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
fn signed<T: Copy>(classes: &[T]) -> Vec<(T, bool)> {
    classes
        .iter()
        .flat_map(|&c| [(c, false), (c, true)])
        .collect()
}

/// A register of four words.
fn from_words(words: [u32; 4]) -> u128 {
    words
        .iter()
        .fold(0u128, |acc, &w| acc << 32 | u128::from(w))
}

/// A register of two doublewords.
fn from_doublewords(dwords: [u64; 2]) -> u128 {
    u128::from(dwords[0]) << 64 | u128::from(dwords[1])
}

const RT: u8 = 3;
const RA: u8 = 4;
const RB: u8 = 5;
const RC: u8 = 6;

/// One single-precision slot: its cell and its operand words.
type SingleSlot = (String, [u32; 3]);
/// One double-precision slot: its cell, its operands, and its mode.
type DoubleSlot = (String, [u64; 3], Mode);

struct Builder {
    rng: SplitMix,
    vectors: Vec<DocumentedVector>,
}

impl Builder {
    /// The FPSCR a vector starts with.
    ///
    /// Every eighth vector carries random sticky flags, so the expected
    /// flags also test that no instruction clears one.
    fn start_fpscr(&mut self) -> u128 {
        if self.vectors.len() % 8 == 7 {
            (u128::from(self.rng.next()) << 64 | u128::from(self.rng.next())) & FPSCR_DEFINED
        } else {
            0
        }
    }

    fn push(
        &mut self,
        id: String,
        class: VectorClass,
        cells: Vec<String>,
        insns: &[Insn],
        registers: BTreeMap<u8, u128>,
        fpscr: u128,
    ) -> bool {
        match build(id, class, cells, insns, registers, fpscr) {
            Some(vector) => {
                self.vectors.push(vector);
                true
            }
            None => false,
        }
    }

    /// Packs single-precision slots four to a vector of `insn`.
    fn single_vectors(
        &mut self,
        group: &str,
        class: VectorClass,
        insn: Insn,
        slots: Vec<SingleSlot>,
    ) {
        for (n, chunk) in slots.chunks(4).enumerate() {
            let mut operands = [[0u32; 4]; 3];
            for (slot, (_, words)) in chunk.iter().enumerate() {
                for (k, &w) in words.iter().enumerate() {
                    operands[k][slot] = w;
                }
            }
            let registers = [insn.ra, insn.rb, insn.rc]
                .into_iter()
                .zip(operands)
                .take(insn.sources().len())
                .map(|(r, w)| (r, from_words(w)))
                .collect();
            let fpscr = self.start_fpscr();
            let cells = chunk.iter().map(|(cell, _)| cell.clone()).collect();
            let id = format!("{}/{group}/{n:04}", insn.op.mnemonic());
            self.push(id, class, cells, &[insn], registers, fpscr);
        }
    }

    /// Packs double-precision slots two to a vector of `insn`.
    fn double_vectors(
        &mut self,
        group: &str,
        class: VectorClass,
        insn: Insn,
        slots: Vec<DoubleSlot>,
    ) {
        for (n, chunk) in slots.chunks(2).enumerate() {
            let mut operands = [[0u64; 2]; 3];
            let mut fpscr = self.start_fpscr();
            for (slot, (_, dwords, mode)) in chunk.iter().enumerate() {
                for (k, &d) in dwords.iter().enumerate() {
                    operands[k][slot] = d;
                }
                fpscr = with_mode(fpscr, slot, *mode);
            }
            let registers = insn
                .sources()
                .into_iter()
                .zip(operands)
                .map(|(r, d)| (r, from_doublewords(d)))
                .collect();
            let cells = chunk.iter().map(|(cell, ..)| cell.clone()).collect();
            let id = format!("{}/{group}/{n:04}", insn.op.mnemonic());
            self.push(id, class, cells, &[insn], registers, fpscr);
        }
    }
}

/// The documented floating-point list, in a fixed order.
pub fn documented_vectors() -> Vec<DocumentedVector> {
    let mut b = Builder {
        rng: SplitMix(SEED),
        vectors: Vec::new(),
    };
    estimate(&mut b);
    interpolate(&mut b);
    single_arithmetic(&mut b);
    conversion(&mut b);
    double_arithmetic(&mut b);
    round_extend(&mut b);
    sequences(&mut b);
    b.vectors
}

fn single_insn(op: Op) -> Insn {
    match op {
        Op::Fma | Op::Fms | Op::Fnms => Insn::fused(op, RT, RA, RB, RC),
        _ => Insn::new(op, RT, RA, RB),
    }
}

/// [SPU-ISA p:215 s:9] the base fraction's top five bits index the estimate; [SPU-ISA p:217 s:9] frsqest also reads the exponent's parity.
fn estimate(b: &mut Builder) {
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
fn interpolate(b: &mut Builder) {
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
fn single_arithmetic(b: &mut Builder) {
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

/// The scales the conversion vectors cover inside 0 to 127.
pub(crate) const TO_FLOAT_SCALES: [u8; 11] = [0, 1, 2, 23, 24, 31, 32, 64, 100, 126, 127];
pub(crate) const TO_INTEGER_SCALES: [u8; 10] = [0, 1, 8, 23, 24, 31, 32, 64, 126, 127];

/// [SPU-ISA p:220 s:9] csflt and [SPU-ISA p:222 s:9] cuflt: scale = 155 - I8; [SPU-ISA p:221 s:9] cflts and [SPU-ISA p:223 s:9] cfltu: scale = 173 - I8; outside 0 to 127 the result is undefined.
fn conversion(b: &mut Builder) {
    for (op, bias) in [
        (Op::Csflt, TO_FLOAT_SCALE_BIAS),
        (Op::Cuflt, TO_FLOAT_SCALE_BIAS),
    ] {
        let mut integers = vec![
            0,
            1,
            0xFFFF_FFFF,
            2,
            0x7F_FFFF,
            0xFF_FFFF,
            0x100_0000,
            0x100_0001,
            0x7FFF_FFFF,
            0x8000_0000,
            0x8000_0001,
            0xFFFF_FF00,
        ];
        integers.extend((0..4).map(|_| b.rng.next() as u32));
        for &scale in &TO_FLOAT_SCALES {
            let insn = Insn::scaled(op, RT, RA, bias - scale);
            let slots = integers
                .iter()
                .map(|&i| (format!("{}:scale{scale:03}", op.mnemonic()), [i, 0, 0]))
                .collect();
            b.single_vectors(
                &format!("scale{scale:03}"),
                VectorClass::Conversion,
                insn,
                slots,
            );
        }
        undefined_scales(b, op, bias, [1, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF]);
    }
    for (op, bias) in [
        (Op::Cflts, TO_INTEGER_SCALE_BIAS),
        (Op::Cfltu, TO_INTEGER_SCALE_BIAS),
    ] {
        let mut floats = vec![
            0,
            0x8000_0000,
            0x0000_0001,
            0x3F00_0000,
            0x3F80_0000,
            0xBF80_0000,
            0x3FC0_0000,
            0x4B00_0000,
            0x4EFF_FFFF,
            0x4F00_0000,
            0xCF00_0000,
            0xCF00_0001,
            0x4F7F_FFFF,
            0x4F80_0000,
            0x7FFF_FFFF,
            0xFFFF_FFFF,
            0x7F80_0001,
        ];
        floats.extend((0..3).map(|_| b.rng.next() as u32));
        for &scale in &TO_INTEGER_SCALES {
            let insn = Insn::scaled(op, RT, RA, bias - scale);
            let slots = floats
                .iter()
                .map(|&f| (format!("{}:scale{scale:03}", op.mnemonic()), [f, 0, 0]))
                .collect();
            b.single_vectors(
                &format!("scale{scale:03}"),
                VectorClass::Conversion,
                insn,
                slots,
            );
        }
        undefined_scales(b, op, bias, [0x3F80_0000, 0xBF80_0000, 0x4F00_0000, 0]);
    }
}

/// One vector per I8 whose scale falls outside 0 to 127.
fn undefined_scales(b: &mut Builder, op: Op, bias: u8, operands: [u32; 4]) {
    for imm in 0..=255u8 {
        let scale = i32::from(bias) - i32::from(imm);
        if (0..=127).contains(&scale) {
            continue;
        }
        let insn = Insn::scaled(op, RT, RA, imm);
        let registers = BTreeMap::from([(RA, from_words(operands))]);
        let fpscr = b.start_fpscr();
        let cell = format!("{}:undefined-i8-{imm:03}", op.mnemonic());
        let id = format!("{}/undefined-i8/{imm:03}", op.mnemonic());
        b.push(
            id,
            VectorClass::Conversion,
            vec![cell; 4],
            &[insn],
            registers,
            fpscr,
        );
    }
}

/// A double operand with its exponent field in `[low, high]`.
fn double_in(rng: &mut SplitMix, negative: bool, low: u64, high: u64) -> u64 {
    u64::from(negative) << 63 | rng.range(low, high) << 52 | rng.below(1 << 52)
}

/// [SPU-ISA p:197 s:9.2] normal IEEE semantics in four rounding modes; [SPU-ISA p:199 s:9.2.2] UNF is tiny before rounding and inexact.
fn double_arithmetic(b: &mut Builder) {
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
    let (_, a) = super::model::double_operand(x);
    let (_, c) = super::model::double_operand(y);
    super::dyadic::round(&a.mul(&c), super::dyadic::DOUBLE, Mode::Nearest).bits
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
fn round_extend(b: &mut Builder) {
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

/// The sequence kinds, by name.
pub(crate) const SEQUENCE_KINDS: [&str; 5] = [
    "single-chain",
    "double-chain",
    "mixed-slices",
    "sticky",
    "convert-chain",
];

/// The two-instruction sequences.
///
/// In each sequence one of these holds:
/// - a result feeds the next instruction;
/// - the two instructions raise flags in different slices;
/// - both instructions raise the same sticky flag.
///
/// [SPU-ISA p:200 s:9.3] the status bits are sticky and each slice has its own fields.
fn sequences(b: &mut Builder) {
    const NORMAL: [SingleClass; 3] = [SingleClass::Low, SingleClass::Mid, SingleClass::High];
    for kind in SEQUENCE_KINDS {
        let mut made = 0;
        let mut attempts = 0;
        while made < 20 && attempts < 400 {
            attempts += 1;
            let r = &mut b.rng;
            let single = |r: &mut SplitMix| {
                from_words(std::array::from_fn(|_| {
                    NORMAL[r.below(3) as usize].draw(r) | u32::from(r.bit()) << 31
                }))
            };
            let double = |r: &mut SplitMix| {
                from_doublewords(std::array::from_fn(|_| {
                    let class = [DoubleClass::Small, DoubleClass::Mid, DoubleClass::Large]
                        [r.below(3) as usize];
                    class.draw(r) | u64::from(r.bit()) << 63
                }))
            };
            let (insns, registers) = match kind {
                "single-chain" => (
                    vec![Insn::new(Op::Fa, 3, 4, 5), Insn::new(Op::Fm, 7, 3, 6)],
                    vec![(4, single(r)), (5, single(r)), (6, single(r))],
                ),
                "double-chain" => (
                    vec![Insn::new(Op::Dfa, 3, 4, 5), Insn::new(Op::Frds, 7, 3, 0)],
                    vec![(4, double(r)), (5, double(r))],
                ),
                "mixed-slices" => (
                    vec![Insn::new(Op::Fm, 3, 4, 5), Insn::new(Op::Dfm, 7, 8, 9)],
                    vec![
                        (4, single(r)),
                        (5, single(r)),
                        (8, double(r)),
                        (9, double(r)),
                    ],
                ),
                "sticky" => (
                    vec![Insn::new(Op::Fa, 3, 4, 5), Insn::new(Op::Fa, 3, 3, 6)],
                    vec![(4, single(r)), (5, single(r)), (6, single(r))],
                ),
                _ => {
                    let first = TO_FLOAT_SCALE_BIAS - TO_FLOAT_SCALES[r.below(11) as usize];
                    let second = TO_INTEGER_SCALE_BIAS - TO_INTEGER_SCALES[r.below(10) as usize];
                    (
                        vec![
                            Insn::scaled(Op::Csflt, 3, 4, first),
                            Insn::scaled(Op::Cflts, 7, 3, second),
                        ],
                        vec![(4, from_words(std::array::from_fn(|_| r.next() as u32)))],
                    )
                }
            };
            let mut fpscr = (u128::from(r.next()) << 64 | u128::from(r.next())) & FPSCR_DEFINED;
            if r.bit() {
                // Half the sequences start with no sticky flag.
                fpscr &= !FPSCR_DEFINED | with_mode(0, 0, Mode::Down) | with_mode(0, 1, Mode::Down);
            }
            let id = format!("seq/{kind}/{made:04}");
            let cells = vec![format!("seq:{kind}")];
            if b.push(
                id,
                VectorClass::Sequence,
                cells,
                &insns,
                registers.into_iter().collect(),
                fpscr,
            ) {
                made += 1;
            }
        }
    }
}
