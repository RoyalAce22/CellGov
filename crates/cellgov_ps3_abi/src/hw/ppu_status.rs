//! The bit layout of the PPU's status registers: CR field 0, XER and
//! the FPSCR.
//!
//! The bit numbers are the ones the ISA prints: bit 0 is the most
//! significant bit of the 32-bit CR and FPSCR, and XER bit 32 is the
//! most significant bit of its low word.
// [PPC-Book1 p:18 s:2.3.1] CR field 0 is bits 0:3, LT GT EQ SO.
// [PPC-Book1 p:32 s:3.2.2] XER bit 32 is SO, 33 OV, 34 CA, 57:63 the byte count; 0:31 and 35:56 are reserved.
// [PPC-Book1 p:87 s:4.2.2] FPSCR bit 0 is FX, 2 VX, 3 OX, 6 XX, 7 VXSNAN.
// [PPC-Book1 p:88 s:4.2.2] FPSCR bit 13 is FR, 14 FI, 15:19 FPRF.
// [PPC-Book1 p:89 s:4.2.2] FPSCR bit 23 is VXCVI, 29 NI.

/// The mask of `width` bits starting at ISA bit `first` of a 32-bit
/// register.
pub const fn word_field(first: u32, width: u32) -> u32 {
    ((1u32 << width) - 1) << (32 - first - width)
}

/// CR0 LT, GT and EQ.
pub const CR0_LT_GT_EQ: u32 = word_field(0, 3);

/// CR0 LT and GT.
pub const CR0_LT_GT: u32 = word_field(0, 2);

/// CR0 EQ.
pub const CR0_EQ: u32 = word_field(2, 1);

/// CR0 SO.
pub const CR0_SO: u32 = word_field(3, 1);

/// XER SO, bit 32.
pub const XER_SO: u64 = 1 << 31;

/// XER OV, bit 33.
pub const XER_OV: u64 = 1 << 30;

/// XER CA, bit 34.
pub const XER_CA: u64 = 1 << 29;

/// XER bits 57:63, the byte count of the string instructions.
pub const XER_BYTE_COUNT: u64 = 0x7f;

/// The reserved XER bits, 0:31 and 35:56.
pub const XER_RESERVED: u64 = !(XER_SO | XER_OV | XER_CA | XER_BYTE_COUNT);

/// FPSCR FX, the exception summary.
pub const FPSCR_FX: u32 = word_field(0, 1);

/// FPSCR VX, the invalid-operation summary.
pub const FPSCR_VX: u32 = word_field(2, 1);

/// FPSCR OX, overflow.
pub const FPSCR_OX: u32 = word_field(3, 1);

/// FPSCR XX, inexact.
pub const FPSCR_XX: u32 = word_field(6, 1);

/// FPSCR VXSNAN, a signalling NaN operand.
pub const FPSCR_VXSNAN: u32 = word_field(7, 1);

/// FPSCR FR, fraction rounded.
pub const FPSCR_FR: u32 = word_field(13, 1);

/// FPSCR FI, fraction inexact.
pub const FPSCR_FI: u32 = word_field(14, 1);

/// FPSCR FPRF, the result class and condition code.
pub const FPSCR_FPRF: u32 = word_field(15, 5);

/// FPSCR VXCVI, an invalid integer convert.
pub const FPSCR_VXCVI: u32 = word_field(23, 1);

/// FPSCR NI, non-IEEE mode.
pub const FPSCR_NI: u32 = word_field(29, 1);

#[cfg(test)]
#[path = "tests/ppu_status_tests.rs"]
mod tests;
