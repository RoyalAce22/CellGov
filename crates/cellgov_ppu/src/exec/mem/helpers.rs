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
// [PPC-Book1 p:103 s:4.6.2] DOUBLE(WORD): single-precision to double-precision conversion pseudocode (normalized / denormalized / Zero / Infinity / NaN branches).
/// PPC `DOUBLE(WORD)`: 32-bit single -> 64-bit double; preserves NaN
/// payloads bit-exactly so SNaNs survive stfsx -> lfsx round-trips.
pub(super) fn double_word(w: u32) -> u64 {
    let exp = (w >> 23) & 0xFF;
    let frac23 = w & 0x007F_FFFF;
    if exp == 0xFF && frac23 != 0 {
        // NaN: WORD2:31 || 0^29 fills FRT5:63; FRT1:4 inherit WORD1.
        let sign = ((w >> 31) & 1) as u64;
        let frac52 = (frac23 as u64) << 29;
        return (sign << 63) | (0x7FFu64 << 52) | frac52;
    }
    (f32::from_bits(w) as f64).to_bits()
}

// [PPC-Book1 p:106 s:4.6.3] SINGLE(FRS): double-precision to single-precision conversion pseudocode (No Denormalization Required vs Denormalization Required branches).
/// PPC `SINGLE(FRS)`: 64-bit double -> 32-bit single; preserves NaN
/// payloads bit-exactly.
pub(super) fn single_frs(d: u64) -> u32 {
    let exp = ((d >> 52) & 0x7FF) as u32;
    let frac52 = d & 0x000F_FFFF_FFFF_FFFF;
    if exp == 0x7FF && frac52 != 0 {
        // NaN: WORD0:1 <- FRS0:1 (sign + first exp bit = 1);
        // WORD2:31 <- FRS5:34 (rest of exp = 1s + top 23 fraction bits).
        let sign = ((d >> 63) & 1) as u32;
        let frac23 = ((d >> 29) & 0x007F_FFFF) as u32;
        return (sign << 31) | (0xFFu32 << 23) | frac23;
    }
    (f64::from_bits(d) as f32).to_bits()
}
