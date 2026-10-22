//! The seeded draw, the vector builder, and the fixed order of the
//! families.

use std::collections::BTreeMap;

use cellgov_ps3_abi::hw::spu_fpscr::FPSCR_DEFINED;

use super::conversion::conversion;
use super::double::{double_arithmetic, round_extend};
use super::sequences::sequences;
use super::single::{estimate, interpolate, single_arithmetic};
use crate::spu_documented::dyadic::Mode;
use crate::spu_documented::model::{with_mode, Insn, Op};
use crate::spu_documented::question::VectorClass;
use crate::spu_documented::vector::{build, DocumentedVector};

/// SplitMix64: every host draws the same sequence.
pub(super) struct SplitMix(pub(super) u64);

impl SplitMix {
    pub(super) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `[0, n)`; `n` is nonzero.
    pub(super) fn below(&mut self, n: u64) -> u64 {
        ((u128::from(self.next()) * u128::from(n)) >> 64) as u64
    }

    /// A value in `[low, high]`.
    pub(super) fn range(&mut self, low: u64, high: u64) -> u64 {
        low + self.below(high - low + 1)
    }

    pub(super) fn bit(&mut self) -> bool {
        self.next() >> 63 == 1
    }
}

/// The seed of the whole list.
const SEED: u64 = 0x0141_1D0C_F10A_7000;

/// A register of four words.
pub(super) fn from_words(words: [u32; 4]) -> u128 {
    words
        .iter()
        .fold(0u128, |acc, &w| acc << 32 | u128::from(w))
}

/// A register of two doublewords.
pub(super) fn from_doublewords(dwords: [u64; 2]) -> u128 {
    u128::from(dwords[0]) << 64 | u128::from(dwords[1])
}

pub(super) const RT: u8 = 3;
pub(super) const RA: u8 = 4;
pub(super) const RB: u8 = 5;
pub(super) const RC: u8 = 6;

/// One single-precision slot: its cell and its operand words.
type SingleSlot = (String, [u32; 3]);
/// One double-precision slot: its cell, its operands, and its mode.
type DoubleSlot = (String, [u64; 3], Mode);

pub(super) struct Builder {
    pub(super) rng: SplitMix,
    vectors: Vec<DocumentedVector>,
}

impl Builder {
    /// The FPSCR a vector starts with.
    ///
    /// Every eighth vector carries random sticky flags, so the expected
    /// flags also test that no instruction clears one.
    pub(super) fn start_fpscr(&mut self) -> u128 {
        if self.vectors.len() % 8 == 7 {
            (u128::from(self.rng.next()) << 64 | u128::from(self.rng.next())) & FPSCR_DEFINED
        } else {
            0
        }
    }

    pub(super) fn push(
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
    pub(super) fn single_vectors(
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
    pub(super) fn double_vectors(
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

pub(super) fn single_insn(op: Op) -> Insn {
    match op {
        Op::Fma | Op::Fms | Op::Fnms => Insn::fused(op, RT, RA, RB, RC),
        _ => Insn::new(op, RT, RA, RB),
    }
}
