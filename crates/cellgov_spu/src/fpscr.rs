//! Typed access to the FPSCR: the double-precision rounding modes and the
//! sticky exception flags each floating-point operation ORs in.
//!
//! Every status bit is sticky: once set it stays set until `fscrwr`
//! rewrites the register, so the accumulators only ever OR.
// [SPU-ISA p:200 s:9.3] the status bits are sticky until an fscrwr clears them.

use cellgov_float::{Flags, Rounding};
use cellgov_ps3_abi::hw::spu_fpscr::{
    fpscr_field, FPSCR_DBZ_FIRST, FPSCR_DOUBLE_FIRST, FPSCR_RN_FIRST, FPSCR_SINGLE_FIRST,
};

use crate::state::SpuState;

/// The FPSCR bits of the set entries of `bits`, one bit per entry,
/// starting at bit `first`.
fn field_bits(first: u32, bits: &[bool]) -> u128 {
    bits.iter()
        .enumerate()
        .filter(|(_, &set)| set)
        .fold(0, |mask, (offset, _)| {
            mask | fpscr_field(first + offset as u32, 1)
        })
}

impl SpuState {
    /// The rounding modes of the two double-precision slices.
    pub fn fpscr_rounding(&self) -> [Rounding; 2] {
        FPSCR_RN_FIRST.map(|first| {
            match (self.fpscr & fpscr_field(first, 2)) >> (128 - first - 2) {
                0 => Rounding::NearestEven,
                1 => Rounding::TowardZero,
                2 => Rounding::TowardPositive,
                _ => Rounding::TowardNegative,
            }
        })
    }

    /// ORs each single-precision slice's OVF, UNF and DIFF in.
    pub fn fpscr_accumulate_single(&mut self, flags: [Flags; 4]) {
        for (first, flags) in FPSCR_SINGLE_FIRST.into_iter().zip(flags) {
            self.fpscr |= field_bits(first, &[flags.overflow, flags.underflow, flags.diff]);
        }
    }

    /// ORs each double-precision slice's OVF, UNF, INX, INV, NaN and
    /// DENORM in.
    pub fn fpscr_accumulate_double(&mut self, flags: [Flags; 2]) {
        for (first, flags) in FPSCR_DOUBLE_FIRST.into_iter().zip(flags) {
            self.fpscr |= field_bits(
                first,
                &[
                    flags.overflow,
                    flags.underflow,
                    flags.inexact,
                    flags.invalid,
                    flags.nan,
                    flags.denormal,
                ],
            );
        }
    }

    /// Sets the divide-by-zero flag of each single-precision slice that
    /// divided by zero.
    pub fn fpscr_accumulate_dbz(&mut self, divided_by_zero: [bool; 4]) {
        self.fpscr |= field_bits(FPSCR_DBZ_FIRST, &divided_by_zero);
    }
}

#[cfg(test)]
#[path = "tests/fpscr_tests.rs"]
mod tests;
