//! Where the firmware set is placed: the default base and the checked override.

use super::error::FirmwareLoadError;

/// Round `addr` up to the next 4 KiB boundary.
pub(super) fn page_align_up_u64(addr: u64) -> Result<u64, FirmwareLoadError> {
    let rounded = addr
        .checked_add(0xFFF)
        .ok_or(FirmwareLoadError::PageAlignOverflow { addr })?;
    Ok(rounded & !0xFFFu64)
}

/// The PRX placement base: the `prx_base` boot override when the run
/// names one, [`default_prx_base`] otherwise.
///
/// # Errors
///
/// [`checked_prx_base`] refuses the override.
pub(super) fn resolve_prx_base(
    prx_base: Option<u64>,
    code_floor: u32,
) -> Result<u64, FirmwareLoadError> {
    match prx_base {
        Some(base) => checked_prx_base(base, code_floor),
        None => Ok(default_prx_base(code_floor)),
    }
}

/// The first 64K-aligned page at or past `code_floor`.
///
/// Callers must set `code_floor` past every prior allocation in the
/// main region; this function does not validate that.
pub(super) fn default_prx_base(code_floor: u32) -> u64 {
    (u64::from(code_floor) + 0xFFFF) & !0xFFFF
}

/// Check a `prx_base` override against the placements the main region can take.
///
/// # Errors
///
/// - `base` is not 64K-aligned.
/// - `base` is below `code_floor`.
/// - `base` is outside the main region.
pub(super) fn checked_prx_base(base: u64, code_floor: u32) -> Result<u64, FirmwareLoadError> {
    let refuse = |reason: String| FirmwareLoadError::PrxBase { base, reason };
    if base & 0xFFFF != 0 {
        return Err(refuse("must be 64K-aligned (low 16 bits zero)".to_string()));
    }
    if base < code_floor as u64 {
        return Err(refuse(format!("below code_floor 0x{code_floor:x}")));
    }
    // Main region spans `[0, 0x4000_0000)`; PRX placement above that
    // hits reserved or unmapped regions.
    if base >= 0x4000_0000 {
        return Err(refuse("must be in main region (< 0x4000_0000)".to_string()));
    }
    Ok(base)
}
