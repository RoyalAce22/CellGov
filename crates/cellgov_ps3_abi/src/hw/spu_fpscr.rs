//! The SPU floating-point status and control register's bit layout.
//!
//! The FPSCR is 128 bits, numbered as the ISA prints them: bit 0 is the
//! most significant. Every other bit is unused and reads as zero.
// [SPU-ISA p:200 s:9.3] bits 20:23 are rounding control, 29:31 the slice-0 single-precision flags, 50:55 the slice-0 double-precision flags.
// [SPU-ISA p:201 s:9.3] 61:63, 93:95 and 125:127 are the slice 1 to 3 single-precision flags, 82:87 the slice-1 double-precision flags, 116:119 the per-slice divide-by-zero flags.

/// The mask of `width` FPSCR bits starting at bit `first`.
pub const fn fpscr_field(first: u32, width: u32) -> u128 {
    ((1u128 << width) - 1) << (128 - first - width)
}

/// The first bit of each double-precision slice's two-bit rounding field:
/// 00 nearest even, 01 toward zero, 10 toward +infinity, 11 toward
/// -infinity.
pub const FPSCR_RN_FIRST: [u32; 2] = [20, 22];

/// The first bit of each single-precision slice's flags, in the order
/// OVF, UNF, DIFF.
pub const FPSCR_SINGLE_FIRST: [u32; 4] = [29, 61, 93, 125];

/// The first bit of each double-precision slice's flags, in the order
/// OVF, UNF, INX, INV, NaN, DENORM.
pub const FPSCR_DOUBLE_FIRST: [u32; 2] = [50, 82];

/// The divide-by-zero flag of single-precision slice 0; slice `n` is bit
/// `116 + n`.
pub const FPSCR_DBZ_FIRST: u32 = 116;

/// Every defined FPSCR bit; `fscrrd` reads each other bit as zero.
pub const FPSCR_DEFINED: u128 = fpscr_field(FPSCR_RN_FIRST[0], 4)
    | fpscr_field(FPSCR_SINGLE_FIRST[0], 3)
    | fpscr_field(FPSCR_SINGLE_FIRST[1], 3)
    | fpscr_field(FPSCR_SINGLE_FIRST[2], 3)
    | fpscr_field(FPSCR_SINGLE_FIRST[3], 3)
    | fpscr_field(FPSCR_DOUBLE_FIRST[0], 6)
    | fpscr_field(FPSCR_DOUBLE_FIRST[1], 6)
    | fpscr_field(FPSCR_DBZ_FIRST, 4);

#[cfg(test)]
#[path = "tests/spu_fpscr_tests.rs"]
mod tests;
