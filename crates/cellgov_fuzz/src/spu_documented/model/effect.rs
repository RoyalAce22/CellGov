//! The instruction and effect model: the ops, the register and FPSCR
//! helpers, and the effect of one instruction.

use std::collections::BTreeMap;

use cellgov_ps3_abi::hw::spu_fpscr::{FPSCR_DBZ_FIRST, FPSCR_RN_FIRST};
use cellgov_ps3_abi::hw::spu_isa::{TO_FLOAT_SCALE_BIAS, TO_INTEGER_SCALE_BIAS};

use super::double::{double_slot, double_slot_effect, extend_to_double, round_to_single};
use super::single::{
    extended_operand, extended_result, interpolate, single_slot, to_integer, SingleOut,
};
use crate::spu_documented::dyadic::{Dyadic, Mode};
use crate::spu_documented::question::Question;

/// A piece of a result: fixed by a document, or open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Known<T> {
    Value(T),
    Open(Question),
}

/// An FPSCR flag an instruction raises, does not raise, or may raise.
pub(crate) type Flag = Known<bool>;

pub(super) const SET: Flag = Known::Value(true);
pub(super) const CLEAR: Flag = Known::Value(false);

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
pub(super) const SINGLE_FLAGS: &str = "SPU-ISA p:196 s:9.1";
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

pub(super) fn push_write(
    effect: &mut Effect,
    insn: Insn,
    mask: u128,
    piece: String,
    value: Known<u128>,
) {
    effect.writes.push(Write {
        register: insn.rt,
        piece,
        mask,
        value,
        citation: insn.op.citation(),
    });
}
