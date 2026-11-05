//! Byte reversal, the aligned 16-byte vector read, the single/double conversions and the reservation check.

use crate::exec::memory_helpers::{LoadPort, Width};
use crate::state::PpuState;
use cellgov_sync::ReservedLine;

use cellgov_ps3_abi::hw::ppu::CELL_EA_LIMIT;

/// Whether the unit's reservation covers `ea`'s line.
///
/// A reservation comes from a load that succeeded, so its line lies
/// inside the Cell EA space; an `ea` past that space names no held line.
pub(super) fn holds_reservation_for(state: &PpuState, ea: u64) -> bool {
    match state.reservation() {
        Some(line) => ea <= CELL_EA_LIMIT && line.addr() == ReservedLine::containing(ea).addr(),
        None => false,
    }
}

/// Byte-reverse the low `width` bytes of `val` and zero the bytes
/// above them.
#[inline]
pub(super) fn swap_low_bytes(val: u64, width: Width) -> u64 {
    match width {
        Width::B1 => val as u8 as u64,
        Width::B2 => (val as u16).swap_bytes() as u64,
        Width::B4 => (val as u32).swap_bytes() as u64,
        Width::B8 => val.swap_bytes(),
    }
}

/// Resolve a 16-byte aligned vector-line read with store-buffer overlay.
///
/// # Errors
///
/// Returns an `Unmapped` `MemError` when no region view covers the line.
pub(super) fn read_aligned_16(
    port: &mut LoadPort<'_, '_>,
    aligned: u64,
) -> Result<u128, cellgov_mem::MemError> {
    if let Some(v) = port.forward(aligned, 16) {
        return Ok(v);
    }
    let mut bytes = [0u8; 16];
    if !port.read_committed(aligned, &mut bytes) {
        return Err(cellgov_mem::MemError::Unmapped(cellgov_mem::FaultContext {
            addr: aligned,
            nearest_below: None,
            nearest_above: None,
        }));
    }
    port.overlay(aligned, &mut bytes);
    Ok(u128::from_be_bytes(bytes))
}

#[inline]
#[track_caller]
/// PPC `DOUBLE(WORD)`: 32-bit single -> 64-bit double, by bit selection
/// with no rounding. A NaN keeps its payload bit-exactly, so an SNaN
/// survives an stfsx -> lfsx round trip.
///
/// [PPC-Book1 p:103 s:4.6.2] DOUBLE(WORD): single-precision to double-precision conversion pseudocode (normalized / denormalized / Zero / Infinity / NaN branches).
pub(super) fn double_word(w: u32) -> u64 {
    let sign = u64::from(w >> 31) << 63;
    let exp = (w >> 23) & 0xFF;
    let frac = u64::from(w & 0x007F_FFFF);
    match (exp, frac) {
        // Zero: keep only the sign.
        (0, 0) => sign,
        // Denormalized: normalize the fraction, one exponent step per
        // shift, from the single exponent -126.
        (0, _) => {
            let shift = frac.leading_zeros() - (64 - 23) + 1;
            let fraction = (frac << shift) & 0x007F_FFFF;
            let biased = (1023 - 126 - u64::from(shift)) << 52;
            sign | biased | fraction << 29
        }
        // Infinity or NaN: FRT1:4 copy WORD1, which widens the exponent
        // to all ones, and WORD2:31 || 0^29 fills FRT5:63.
        (0xFF, _) => sign | 0x7FF << 52 | frac << 29,
        // Normalized: rebias the exponent and widen the fraction.
        _ => sign | (u64::from(exp) + 1023 - 127) << 52 | frac << 29,
    }
}

/// PPC `SINGLE(FRS)`: the word a single-precision store writes. A bit
/// selection with no rounding: a double above the single range stores
/// a finite word unequal to it, and fraction bits below single
/// precision drop.
///
/// [PPC-Book1 p:106 s:4.6.3] SINGLE(FRS): double-precision to single-precision conversion pseudocode (No Denormalization Required vs Denormalization Required branches).
pub(super) fn single_frs(d: u64) -> u32 {
    let exp = ((d >> 52) & 0x7FF) as u32;
    let sign = (d >> 63) as u32;
    if exp > 896 || d & !(1 << 63) == 0 {
        // WORD0:1 <- FRS0:1; WORD2:31 <- FRS5:34.
        return (((d >> 62) as u32) << 30) | ((d >> 29) as u32 & 0x3FFF_FFFF);
    }
    if exp >= 874 {
        // frac <- 0b1 || FRS12:63, shifted right until exp = -126;
        // WORD9:31 <- frac1:23.
        let frac = (1u64 << 52) | (d & 0x000F_FFFF_FFFF_FFFF);
        let shifted = frac >> (897 - exp);
        return (sign << 31) | ((shifted >> 29) as u32 & 0x007F_FFFF);
    }
    // The book leaves WORD undefined for a nonzero double below biased
    // exponent 874. The reference console stores the signed zero
    // (`ppu_stfs_select`).
    sign << 31
}

#[cfg(test)]
#[path = "tests/single_frs_tests.rs"]
mod single_frs_tests;

#[cfg(test)]
#[path = "tests/double_word_tests.rs"]
mod double_word_tests;

#[cfg(test)]
#[path = "tests/single_frs_console_tests.rs"]
mod single_frs_console_tests;
