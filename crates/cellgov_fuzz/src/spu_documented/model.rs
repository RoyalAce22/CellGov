//! What the documents say each floating-point instruction does: the
//! register pieces it writes and the FPSCR flags it raises, each either
//! fixed by a page or left open with a named question.

use std::collections::BTreeMap;

use cellgov_ps3_abi::hw::spu_fpscr::{
    FPSCR_DBZ_FIRST, FPSCR_DOUBLE_FIRST, FPSCR_RN_FIRST, FPSCR_SINGLE_FIRST,
};
use cellgov_ps3_abi::hw::spu_isa::{
    SPU_DEFAULT_QNAN_F64, TO_FLOAT_SCALE_BIAS, TO_INTEGER_SCALE_BIAS,
};

use super::dyadic::{round, Dyadic, Mode, Natural, DOUBLE, SINGLE};
use super::question::Question;

/// A piece of a result: fixed by a document, or open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Known<T> {
    Value(T),
    Open(Question),
}

/// An FPSCR flag an instruction raises, does not raise, or may raise.
pub(crate) type Flag = Known<bool>;

const SET: Flag = Known::Value(true);
const CLEAR: Flag = Known::Value(false);

/// The floating-point instructions the list covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Op {
    Fa,
    Fs,
    Fm,
    Fma,
    Fms,
    Fnms,
    Frest,
    Frsqest,
    Fi,
    Csflt,
    Cuflt,
    Cflts,
    Cfltu,
    Dfa,
    Dfs,
    Dfm,
    Dfma,
    Dfms,
    Dfnms,
    Dfnma,
    Frds,
    Fesd,
}

impl Op {
    pub(crate) fn mnemonic(self) -> &'static str {
        match self {
            Self::Fa => "fa",
            Self::Fs => "fs",
            Self::Fm => "fm",
            Self::Fma => "fma",
            Self::Fms => "fms",
            Self::Fnms => "fnms",
            Self::Frest => "frest",
            Self::Frsqest => "frsqest",
            Self::Fi => "fi",
            Self::Csflt => "csflt",
            Self::Cuflt => "cuflt",
            Self::Cflts => "cflts",
            Self::Cfltu => "cfltu",
            Self::Dfa => "dfa",
            Self::Dfs => "dfs",
            Self::Dfm => "dfm",
            Self::Dfma => "dfma",
            Self::Dfms => "dfms",
            Self::Dfnms => "dfnms",
            Self::Dfnma => "dfnma",
            Self::Frds => "frds",
            Self::Fesd => "fesd",
        }
    }

    /// The instruction's own page.
    pub(crate) fn citation(self) -> &'static str {
        match self {
            Self::Fa => "SPU-ISA p:202 s:9",
            Self::Dfa => "SPU-ISA p:203 s:9",
            Self::Fs => "SPU-ISA p:204 s:9",
            Self::Dfs => "SPU-ISA p:205 s:9",
            Self::Fm => "SPU-ISA p:206 s:9",
            Self::Dfm => "SPU-ISA p:207 s:9",
            Self::Fma => "SPU-ISA p:208 s:9",
            Self::Dfma => "SPU-ISA p:209 s:9",
            Self::Fnms => "SPU-ISA p:210 s:9",
            Self::Dfnms => "SPU-ISA p:211 s:9",
            Self::Fms => "SPU-ISA p:212 s:9",
            Self::Dfms => "SPU-ISA p:213 s:9",
            Self::Dfnma => "SPU-ISA p:214 s:9",
            Self::Frest => "SPU-ISA p:215 s:9",
            Self::Frsqest => "SPU-ISA p:217 s:9",
            Self::Fi => "SPU-ISA p:219 s:9",
            Self::Csflt => "SPU-ISA p:220 s:9",
            Self::Cflts => "SPU-ISA p:221 s:9",
            Self::Cuflt => "SPU-ISA p:222 s:9",
            Self::Cfltu => "SPU-ISA p:223 s:9",
            Self::Frds => "SPU-ISA p:224 s:9",
            Self::Fesd => "SPU-ISA p:225 s:9",
        }
    }
}

/// One instruction with its register fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Insn {
    pub(crate) op: Op,
    pub(crate) rt: u8,
    pub(crate) ra: u8,
    pub(crate) rb: u8,
    pub(crate) rc: u8,
    pub(crate) imm: u8,
}

impl Insn {
    pub(crate) fn new(op: Op, rt: u8, ra: u8, rb: u8) -> Self {
        Self {
            op,
            rt,
            ra,
            rb,
            rc: 0,
            imm: 0,
        }
    }

    pub(crate) fn fused(op: Op, rt: u8, ra: u8, rb: u8, rc: u8) -> Self {
        Self {
            rc,
            ..Self::new(op, rt, ra, rb)
        }
    }

    pub(crate) fn scaled(op: Op, rt: u8, ra: u8, imm: u8) -> Self {
        Self {
            imm,
            ..Self::new(op, rt, ra, 0)
        }
    }

    /// The instruction word, from the opcode fields on each
    /// instruction's page.
    pub(crate) fn encode(self) -> u32 {
        let [rt, ra, rb, rc] = [self.rt, self.ra, self.rb, self.rc].map(|r| u32::from(r & 0x7F));
        let rr = |opcode: u32| opcode << 21 | rb << 14 | ra << 7 | rt;
        let rrr = |opcode: u32| opcode << 28 | rt << 21 | rb << 14 | ra << 7 | rc;
        let ri8 = |opcode: u32| opcode << 22 | u32::from(self.imm) << 14 | ra << 7 | rt;
        match self.op {
            Op::Fa => rr(0b010_1100_0100),
            Op::Fs => rr(0b010_1100_0101),
            Op::Fm => rr(0b010_1100_0110),
            Op::Dfa => rr(0b010_1100_1100),
            Op::Dfs => rr(0b010_1100_1101),
            Op::Dfm => rr(0b010_1100_1110),
            Op::Dfma => rr(0b011_0101_1100),
            Op::Dfms => rr(0b011_0101_1101),
            Op::Dfnms => rr(0b011_0101_1110),
            Op::Dfnma => rr(0b011_0101_1111),
            Op::Frest => rr(0b001_1011_1000),
            Op::Frsqest => rr(0b001_1011_1001),
            Op::Fi => rr(0b011_1101_0100),
            Op::Frds => rr(0b011_1011_1001),
            Op::Fesd => rr(0b011_1011_1000),
            Op::Fma => rrr(0b1110),
            Op::Fnms => rrr(0b1101),
            Op::Fms => rrr(0b1111),
            Op::Csflt => ri8(0b01_1101_1010),
            Op::Cflts => ri8(0b01_1101_1000),
            Op::Cuflt => ri8(0b01_1101_1011),
            Op::Cfltu => ri8(0b01_1101_1001),
        }
    }

    /// The registers the instruction reads.
    pub(crate) fn sources(self) -> Vec<u8> {
        match self.op {
            Op::Fa | Op::Fs | Op::Fm | Op::Dfa | Op::Dfs | Op::Dfm | Op::Fi => {
                vec![self.ra, self.rb]
            }
            Op::Fma | Op::Fms | Op::Fnms => vec![self.ra, self.rb, self.rc],
            Op::Dfma | Op::Dfms | Op::Dfnms | Op::Dfnma => vec![self.ra, self.rb, self.rt],
            Op::Frest
            | Op::Frsqest
            | Op::Csflt
            | Op::Cuflt
            | Op::Cflts
            | Op::Cfltu
            | Op::Frds
            | Op::Fesd => vec![self.ra],
        }
    }
}

/// A piece of a register the instruction writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Write {
    pub(crate) register: u8,
    /// The piece's name within the register, as `w0` or `d1.payload`.
    pub(crate) piece: String,
    /// The bits of the 128-bit register the piece covers.
    pub(crate) mask: u128,
    pub(crate) value: Known<u128>,
    pub(crate) citation: &'static str,
}

/// An FPSCR flag the instruction may set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FlagWrite {
    pub(crate) name: String,
    /// The bit, numbered from 0 at the most significant end.
    pub(crate) bit: u32,
    pub(crate) raised: Flag,
    pub(crate) citation: &'static str,
}

/// What one instruction does to the register file and the FPSCR.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Effect {
    pub(crate) writes: Vec<Write>,
    pub(crate) flags: Vec<FlagWrite>,
}

/// [SPU-ISA p:196 s:9.1] OVF, UNF, DIFF, DBZ.
const SINGLE_FLAGS: &str = "SPU-ISA p:196 s:9.1";
/// [SPU-ISA p:198 s:9.2.2] the IEEE exception conditions.
const DOUBLE_FLAGS: &str = "SPU-ISA p:198 s:9.2.2";
/// [SPU-ISA p:200 s:9.3] Table 9-5: the flags fesd and frds can set.
const DOUBLE_TABLE: &str = "SPU-ISA p:200 s:9.3";

/// The mask of word slot `slot` in a 128-bit register.
pub(crate) fn word_mask(slot: usize) -> u128 {
    0xFFFF_FFFF_u128 << (96 - 32 * slot)
}

pub(crate) fn word(register: u128, slot: usize) -> u32 {
    (register >> (96 - 32 * slot)) as u32
}

pub(crate) fn doubleword(register: u128, slot: usize) -> u64 {
    (register >> (64 - 64 * slot)) as u64
}

/// The bit of the FPSCR, numbered from 0 at the most significant end.
pub(crate) fn fpscr_bit(bit: u32) -> u128 {
    1u128 << (127 - bit)
}

/// The rounding mode for double slot `slot`.
///
/// [SPU-ISA p:197 s:9.2] RN0 controls slice 0 and RN1 slice 1.
pub(crate) fn slot_mode(fpscr: u128, slot: usize) -> Mode {
    Mode::from_field((fpscr >> (126 - FPSCR_RN_FIRST[slot])) as u32)
}

/// The FPSCR with `mode` in double slot `slot`.
pub(crate) fn with_mode(fpscr: u128, slot: usize, mode: Mode) -> u128 {
    let shift = 126 - FPSCR_RN_FIRST[slot];
    fpscr & !(3u128 << shift) | u128::from(mode.field()) << shift
}

/// The effect of `insn` on registers `regs` under `fpscr`. A register
/// missing from `regs` holds zero.
pub(crate) fn effect(insn: Insn, regs: &BTreeMap<u8, u128>, fpscr: u128) -> Effect {
    let reg = |r: u8| regs.get(&r).copied().unwrap_or(0);
    let mut effect = Effect::default();
    match insn.op {
        Op::Fa | Op::Fs | Op::Fm | Op::Fma | Op::Fms | Op::Fnms => {
            let (a, b, c) = (reg(insn.ra), reg(insn.rb), reg(insn.rc));
            for slot in 0..4 {
                let operands = [a, b, c].map(|r| extended_operand(word(r, slot)));
                let [x, y, z] = operands.clone().map(|(value, _)| value);
                let exact = match insn.op {
                    Op::Fa => x.add(&y),
                    Op::Fs => x.add(&y.negated()),
                    Op::Fm => x.mul(&y),
                    Op::Fma => x.mul(&y).add(&z),
                    Op::Fms => x.mul(&y).add(&z.negated()),
                    _ => z.add(&x.mul(&y).negated()),
                };
                let used = if matches!(insn.op, Op::Fa | Op::Fs | Op::Fm) {
                    2
                } else {
                    3
                };
                let input_diff = operands[..used].iter().any(|(_, diff)| *diff);
                single_slot(&mut effect, insn, slot, extended_result(&exact, input_diff));
            }
        }
        Op::Frest | Op::Frsqest => {
            let a = reg(insn.ra);
            for slot in 0..4 {
                let open = Known::Open(Question::EstimateTables);
                if insn.op == Op::Frsqest {
                    // [SPU-ISA p:217 s:9] the sign bit of the result is zero.
                    push_write(
                        &mut effect,
                        insn,
                        word_mask(slot) & sign_word(slot),
                        format!("w{slot}.sign"),
                        Known::Value(0),
                    );
                    push_write(
                        &mut effect,
                        insn,
                        word_mask(slot) & !sign_word(slot),
                        format!("w{slot}.magnitude"),
                        open,
                    );
                } else {
                    push_write(&mut effect, insn, word_mask(slot), format!("w{slot}"), open);
                }
                // [SPU-ISA p:215 s:9] a zero exponent flags divide by zero.
                let zero_exponent = word(a, slot) >> 23 & 0xFF == 0;
                effect.flags.push(FlagWrite {
                    name: format!("dbz{slot}"),
                    bit: FPSCR_DBZ_FIRST + slot as u32,
                    raised: Known::Value(zero_exponent),
                    citation: SINGLE_FLAGS,
                });
            }
        }
        Op::Fi => {
            let (a, b) = (reg(insn.ra), reg(insn.rb));
            for slot in 0..4 {
                single_slot(
                    &mut effect,
                    insn,
                    slot,
                    interpolate(word(a, slot), word(b, slot)),
                );
            }
        }
        Op::Csflt | Op::Cuflt => {
            let a = reg(insn.ra);
            // [SPU-ISA p:220 s:9] scale = 155 - I8, defined from 0 to 127.
            let scale = i64::from(TO_FLOAT_SCALE_BIAS) - i64::from(insn.imm);
            for slot in 0..4 {
                let out = if (0..=127).contains(&scale) {
                    let w = word(a, slot);
                    let (negative, magnitude) = if insn.op == Op::Csflt {
                        ((w as i32) < 0, (w as i32).unsigned_abs())
                    } else {
                        (false, w)
                    };
                    let exact = Dyadic::new(negative, u128::from(magnitude), -scale);
                    extended_result(&exact, false)
                } else {
                    SingleOut::open(Question::ConversionScale)
                };
                single_slot(&mut effect, insn, slot, out);
            }
        }
        Op::Cflts | Op::Cfltu => {
            let a = reg(insn.ra);
            // [SPU-ISA p:221 s:9] scale = 173 - I8, defined from 0 to 127.
            let scale = i64::from(TO_INTEGER_SCALE_BIAS) - i64::from(insn.imm);
            for slot in 0..4 {
                let value = if (0..=127).contains(&scale) {
                    Known::Value(
                        u128::from(to_integer(word(a, slot), scale, insn.op == Op::Cflts))
                            << (96 - 32 * slot),
                    )
                } else {
                    Known::Open(Question::ConversionScale)
                };
                push_write(
                    &mut effect,
                    insn,
                    word_mask(slot),
                    format!("w{slot}"),
                    value,
                );
            }
        }
        Op::Dfa | Op::Dfs | Op::Dfm | Op::Dfma | Op::Dfms | Op::Dfnms | Op::Dfnma => {
            let (a, b, t) = (reg(insn.ra), reg(insn.rb), reg(insn.rt));
            for slot in 0..2 {
                let out = double_slot(
                    insn.op,
                    [a, b, t].map(|r| doubleword(r, slot)),
                    slot_mode(fpscr, slot),
                );
                double_slot_effect(&mut effect, insn, slot, out, DOUBLE_FLAGS);
            }
        }
        Op::Frds => {
            let a = reg(insn.ra);
            for slot in 0..2 {
                let out = round_to_single(doubleword(a, slot), slot_mode(fpscr, slot));
                double_slot_effect(&mut effect, insn, slot, out, DOUBLE_TABLE);
            }
        }
        Op::Fesd => {
            let a = reg(insn.ra);
            for slot in 0..2 {
                let mut out = extend_to_double((doubleword(a, slot) >> 32) as u32);
                // [SPU-ISA p:200 s:9.3] Table 9-5: fesd never sets OVF, UNF or INX.
                out.flags.truncate_to(&[3, 4, 5]);
                double_slot_effect(&mut effect, insn, slot, out, DOUBLE_TABLE);
            }
        }
    }
    effect
}

fn sign_word(slot: usize) -> u128 {
    0x8000_0000_u128 << (96 - 32 * slot)
}

fn push_write(effect: &mut Effect, insn: Insn, mask: u128, piece: String, value: Known<u128>) {
    effect.writes.push(Write {
        register: insn.rt,
        piece,
        mask,
        value,
        citation: insn.op.citation(),
    });
}

/// A single-precision slot: its result word and its three flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SingleOut {
    pub(crate) bits: Known<u32>,
    pub(crate) overflow: Flag,
    pub(crate) underflow: Flag,
    pub(crate) diff: Flag,
}

impl SingleOut {
    fn open(question: Question) -> Self {
        let open = Known::Open(question);
        Self {
            bits: Known::Open(question),
            overflow: open,
            underflow: open,
            diff: open,
        }
    }
}

fn single_slot(effect: &mut Effect, insn: Insn, slot: usize, out: SingleOut) {
    let value = match out.bits {
        Known::Value(bits) => Known::Value(u128::from(bits) << (96 - 32 * slot)),
        Known::Open(q) => Known::Open(q),
    };
    push_write(effect, insn, word_mask(slot), format!("w{slot}"), value);
    let first = FPSCR_SINGLE_FIRST[slot];
    for (offset, (name, raised)) in [
        ("ovf", out.overflow),
        ("unf", out.underflow),
        ("diff", out.diff),
    ]
    .into_iter()
    .enumerate()
    {
        effect.flags.push(FlagWrite {
            name: format!("s{slot}.{name}"),
            bit: first + offset as u32,
            raised,
            citation: SINGLE_FLAGS,
        });
    }
}

/// A single-precision register word as an extended-range operand, and
/// whether it sets DIFF as an input.
///
/// [SPU-ISA p:195 s:9.1] exponent 255 is an ordinary binade; [SPU-ISA p:196 s:9.1] a denorm operand is a zero, and DIFF marks an input with a maximal exponent or with a zero exponent and a nonzero fraction.
pub(crate) fn extended_operand(word: u32) -> (Dyadic, bool) {
    let exponent = word >> 23 & 0xFF;
    let fraction = word & 0x7F_FFFF;
    if exponent == 0 {
        return (Dyadic::zero(false), fraction != 0);
    }
    (
        Dyadic::new(
            word >> 31 == 1,
            u128::from(fraction | 0x80_0000),
            i64::from(exponent) - 127 - 23,
        ),
        exponent == 255,
    )
}

/// An exact result in extended-range single precision.
///
/// [SPU-ISA p:195 s:9.1] a zero result is +0, and a magnitude above the largest number gives the largest number with the sign.
/// [SPU-ISA p:196 s:9.1] truncation is the only rounding; OVF above Smax before rounding saturates to Smax, UNF below Smin gives +0, and DIFF marks a maximal-exponent result and an underflow.
pub(crate) fn extended_result(exact: &Dyadic, input_diff: bool) -> SingleOut {
    let input_diff = Known::Value(input_diff);
    if exact.is_zero() {
        return SingleOut {
            bits: Known::Value(0),
            overflow: CLEAR,
            underflow: CLEAR,
            diff: input_diff,
        };
    }
    let sign = if exact.negative { 0x8000_0000 } else { 0 };
    let leading = exact.leading_exponent();
    if leading >= 129 {
        return SingleOut {
            bits: Known::Value(sign | 0x7FFF_FFFF),
            overflow: SET,
            underflow: CLEAR,
            diff: SET,
        };
    }
    if leading < -126 {
        return SingleOut {
            bits: Known::Value(0),
            overflow: CLEAR,
            underflow: SET,
            diff: SET,
        };
    }
    let (kept, dropped) = exact.scaled(23 - leading).truncated_magnitude();
    let kept = kept
        .to_u128()
        .expect("invariant: a truncated significand has 24 bits") as u32;
    let biased = (leading + 127) as u32;
    let overflow = if leading == 128 && kept == 0xFF_FFFF && dropped {
        Known::Open(Question::SingleOverflowBand)
    } else {
        CLEAR
    };
    SingleOut {
        bits: Known::Value(sign | biased << 23 | kept & 0x7F_FFFF),
        overflow,
        underflow: CLEAR,
        diff: if biased == 255 { SET } else { input_diff },
    }
}

/// `fi` of RA word `a` and RB word `b`.
///
/// [SPU-ISA p:219 s:9] RT = (-1)^S x (1.BaseFraction - 0.000StepFraction x Y) x 2^(BiasedExponent - 127), Y = `0.RA[13:31]`, with RB laid out as on page 215; [SPU-ISA p:196 s:9.1] fi truncates and sets OVF, UNF and DIFF.
pub(crate) fn interpolate(a: u32, b: u32) -> SingleOut {
    let exponent = b >> 23 & 0xFF;
    if exponent == 0 {
        return SingleOut::open(Question::FiOperandFields);
    }
    let base = u128::from(b >> 10 & 0x1FFF);
    let step = u128::from(b & 0x3FF);
    let y = u128::from(a & 0x7_FFFF);
    // In units of 2^-32: 1.BaseFraction is (2^13 + base) x 2^19, and
    // 0.000StepFraction x Y is step x y.
    let value = ((0x2000 | base) << 19) - step * y;
    let exact = Dyadic::new(b >> 31 == 1, value, i64::from(exponent) - 127 - 32);
    let mut out = extended_result(&exact, exponent == 255);
    let ra_exponent = a >> 23 & 0xFF;
    let ra_reads_as_flagged = ra_exponent == 255 || ra_exponent == 0 && a & 0x7F_FFFF != 0;
    if ra_reads_as_flagged && out.diff == CLEAR {
        out.diff = Known::Open(Question::FiOperandFields);
    }
    out
}

/// cflts (`signed`) or cfltu of one word at `scale`.
///
/// [SPU-ISA p:221 s:9] the value times 2^scale, saturated to [-2^31, 2^31 - 1]; [SPU-ISA p:223 s:9] cfltu saturates to [0, 2^32 - 1]; [SPU-ISA p:196 s:9.1] truncation is the only rounding and a denorm is a zero.
pub(crate) fn to_integer(word: u32, scale: i64, signed: bool) -> u32 {
    let (value, _) = extended_operand(word);
    let (magnitude, _) = value.scaled(scale).truncated_magnitude();
    let limit =
        |bound: u128| magnitude.cmp(&Natural::from_u128(bound)) == std::cmp::Ordering::Greater;
    match (signed, value.negative && !value.is_zero()) {
        (true, false) if limit(0x7FFF_FFFF) => 0x7FFF_FFFF,
        (true, true) if limit(0x8000_0000) => 0x8000_0000,
        (true, negative) => {
            let m = magnitude
                .to_u128()
                .expect("invariant: checked against 2^31") as u32;
            if negative {
                m.wrapping_neg()
            } else {
                m
            }
        }
        (false, true) => 0,
        (false, false) if limit(0xFFFF_FFFF) => 0xFFFF_FFFF,
        (false, false) => magnitude
            .to_u128()
            .expect("invariant: checked against 2^32") as u32,
    }
}

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
    fn truncate_to(&mut self, indices: &[usize]) {
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

fn double_slot_effect(
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
