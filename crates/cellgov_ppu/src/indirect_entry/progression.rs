//! The values of a lost register that a loop steps by one constant: a
//! residue class of the dropped constants.
//!
//! A word that adds the step `s` to a value `d` gives `d + s`. A loop can
//! add `s` any number of times, and 64-bit sums wrap. So a register that
//! holds `d`, and that only words which add `s` change, can hold each
//! `d + k * s` with `k` from 0. These are the values with the low-order
//! `t` bits of `d`, where `t` is the number of trailing zero bits of `s`.
//! The scan does not read the condition that ends a loop, so it bounds
//! no count of passes, and the class holds each value of the register.
//!
//! [PPC-Book1 p:51 s:3.3.8] `addi` and `addis` add EXTS(SI) or EXTS(SI || 0x0000) to RA, or to 0 when RA is 0.
//! [PPC-Book1 p:15 s:1.12.2] Effective address arithmetic wraps from 2^64 - 1 to 0 in 64-bit mode, and from 2^32 - 1 to 0 in 32-bit mode.

use crate::hypercall_cfg::Code;

/// The most low-order bits that a [`Residue`] keeps.
///
/// An indirect branch in 32-bit mode uses only the low-order 32 bits of
/// its target, and a real alias differs only in EA bits 0 and 1. Neither
/// changes the low-order 32 bits. So each word that a branch to a value
/// of the class can enter, in each mode, is a word of the class.
///
/// [PPC-Book1 p:20 s:2.4.1] In 32-bit mode a branch sets the high-order 32 bits of its target address to 0.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
const KEPT_BITS: u32 = 32;

/// The fewest low-order bits that a [`Residue`] needs.
///
/// An indirect branch ignores bits 62 and 63 of its target. So a class
/// of fewer bits holds each word of the code, and gives the bound nothing
/// that a lost value does not give.
///
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
const FEWEST_BITS: u32 = 3;

/// The low-order bits that each constant of a set shares with the first
/// constant of the set.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Congruence {
    /// The first constant, or `None` before the set has one.
    first: Option<u64>,
    /// The number of low-order bits that each constant shares with
    /// `first`, 64 for a set of one constant.
    bits: u32,
}

impl Congruence {
    /// Adds `constant` to the set.
    pub(super) fn note(&mut self, constant: u64) {
        match self.first {
            None => (self.first, self.bits) = (Some(constant), u64::BITS),
            Some(first) => self.bits = self.bits.min((constant ^ first).trailing_zeros()),
        }
    }

    /// Whether the set shares too few low-order bits for a [`Residue`],
    /// so that a constant more cannot change what the set gives.
    pub(super) fn is_spread(&self) -> bool {
        self.first.is_some() && self.bits < FEWEST_BITS
    }

    /// The class of each value `d + k * step`, with `d` a constant of the
    /// set and `k` from 0, or `None` when the class holds each word.
    ///
    /// A `step` of 0 gives the class of the constants themselves.
    pub(super) fn residue(&self, step: i64) -> Option<Residue> {
        let first = self.first?;
        let bits = self
            .bits
            .min(step.cast_unsigned().trailing_zeros())
            .min(KEPT_BITS);
        if bits < FEWEST_BITS {
            return None;
        }
        Some(Residue {
            low: first & low_mask(bits) & !0b11,
            bits,
        })
    }
}

/// The mask of the low-order `bits` bits, for `bits` below 64.
fn low_mask(bits: u32) -> u64 {
    (1u64 << bits) - 1
}

/// The words whose addresses have the low-order bits `low`, which are the
/// words that an indirect branch to a value of the class can enter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Residue {
    /// The low-order `bits` bits of each word address, with bits 62 and
    /// 63 at 0.
    low: u64,
    /// The number of low-order bits, from [`FEWEST_BITS`] to
    /// [`KEPT_BITS`].
    bits: u32,
}

impl Residue {
    /// The address of each word of `code` in the class.
    pub(super) fn words(self, code: &Code) -> impl Iterator<Item = u64> + '_ {
        let mask = low_mask(self.bits);
        let stride = 1usize << (self.bits - 2);
        code.segments().iter().flat_map(move |segment| {
            let delta = self.low.wrapping_sub(segment.vaddr) & mask;
            let first = usize::try_from(delta / 4)
                .ok()
                .filter(|_| delta.is_multiple_of(4))
                .unwrap_or(segment.words.len());
            (first..segment.words.len())
                .step_by(stride)
                .map(move |word| segment.vaddr + (word as u64) * 4)
        })
    }
}
