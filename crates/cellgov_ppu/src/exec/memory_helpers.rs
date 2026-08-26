//! Shared load / store helpers used by the per-form execute arms.
//!
//! `load_ze` / `load_se` overlay buffered stores onto the region view
//! so multi-store stitching (eight `stb`s read as one `ld`) and partial
//! overlaps with pre-block memory both resolve correctly.

use crate::exec::verdict::ExecuteVerdict;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;
use cellgov_mem::RegionView;

/// Linear search for `[ea, ea+len)` covered by one region view.
///
/// O(n) over `region_views`; n is small (single-digit) per dispatch.
/// A hit on a provisional view is logged with that view's memory.
#[inline]
pub(crate) fn load_slice<'a>(
    region_views: &[RegionView<'a>],
    ea: u64,
    len: usize,
) -> Option<&'a [u8]> {
    let end = ea.checked_add(len as u64)?;
    for view in region_views {
        let region_end = view.base + view.bytes.len() as u64;
        if ea >= view.base && end <= region_end {
            view.note_read(ea, len as u32);
            let offset = (ea - view.base) as usize;
            return Some(&view.bytes[offset..offset + len]);
        }
    }
    None
}

/// Synthesize a `MemError::Unmapped` for `ea` with no nearest-region
/// labels populated; the helper does not have a `GuestMemory`
/// reference to walk, only the flat region-view slice.
#[inline]
fn unmapped(ea: u64) -> cellgov_mem::MemError {
    cellgov_mem::MemError::Unmapped(cellgov_mem::FaultContext {
        addr: ea,
        nearest_below: None,
        nearest_above: None,
    })
}

/// The widths a fixed-point load can ask for. Typed so a helper is
/// never handed a size it has no arm for: a caller mistake is a
/// compile error, not a fabricated unmapped fault at run time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoadWidth {
    /// One byte.
    B1,
    /// Halfword.
    B2,
    /// Word.
    B4,
    /// Doubleword.
    B8,
}

impl LoadWidth {
    pub(crate) const fn bytes(self) -> u8 {
        match self {
            Self::B1 => 1,
            Self::B2 => 2,
            Self::B4 => 4,
            Self::B8 => 8,
        }
    }
}

/// Zero-extending load with store-buffer forwarding.
///
/// Slow path overlays buffered stores onto the region view, so
/// multi-store stitching (eight `stb`s read as one `ld`) and partial
/// overlaps with pre-block memory both resolve correctly.
#[inline]
pub(crate) fn load_ze(
    region_views: &[cellgov_mem::RegionView<'_>],
    store_buf: &StoreBuffer,
    ea: u64,
    width: LoadWidth,
) -> Result<u64, cellgov_mem::MemError> {
    let size = width.bytes();
    if let Some(val) = store_buf.forward(ea, size) {
        return Ok(val as u64);
    }
    let slice = load_slice(region_views, ea, size as usize).ok_or_else(|| unmapped(ea))?;
    let mut bytes = [0u8; 8];
    let n = size as usize;
    bytes[..n].copy_from_slice(&slice[..n]);
    store_buf.overlay_range(ea, &mut bytes[..n]);
    Ok(match width {
        LoadWidth::B1 => bytes[0] as u64,
        LoadWidth::B2 => u16::from_be_bytes([bytes[0], bytes[1]]) as u64,
        LoadWidth::B4 => u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as u64,
        LoadWidth::B8 => u64::from_be_bytes(bytes),
    })
}

/// Sign-extending load with store-buffer forwarding. See [`load_ze`].
#[inline]
pub(crate) fn load_se(
    region_views: &[cellgov_mem::RegionView<'_>],
    store_buf: &StoreBuffer,
    ea: u64,
    width: LoadWidth,
) -> Result<u64, cellgov_mem::MemError> {
    let size = width.bytes();
    if let Some(val) = store_buf.forward(ea, size) {
        // `forward` right-aligns `size` bytes; sign must come from
        // the size's MSB, not u64 bit 63 (always 0 for sub-doubleword).
        return Ok(match width {
            LoadWidth::B1 => (val as u8 as i8) as i64 as u64,
            LoadWidth::B2 => (val as u16 as i16) as i64 as u64,
            LoadWidth::B4 => (val as u32 as i32) as i64 as u64,
            LoadWidth::B8 => val as u64,
        });
    }
    let slice = load_slice(region_views, ea, size as usize).ok_or_else(|| unmapped(ea))?;
    let mut bytes = [0u8; 8];
    let n = size as usize;
    bytes[..n].copy_from_slice(&slice[..n]);
    store_buf.overlay_range(ea, &mut bytes[..n]);
    Ok(match width {
        LoadWidth::B1 => (bytes[0] as i8) as i64 as u64,
        LoadWidth::B2 => i16::from_be_bytes([bytes[0], bytes[1]]) as i64 as u64,
        LoadWidth::B4 => i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i64 as u64,
        LoadWidth::B8 => u64::from_be_bytes(bytes),
    })
}

/// Stage a store and drop any same-unit reservation overlapping
/// the written range.
///
/// Reservation clearing is intra-step so a later `stwcx` in the
/// same block observes the invalidation pre-commit.
// [PPC-Book2 p:10 s:1.7.3.1] reservation lost when any store hits the reservation granule.
#[inline]
pub(crate) fn buffer_store(
    store_buf: &mut StoreBuffer,
    state: &mut PpuState,
    ea: u64,
    size: u8,
    value: u64,
) -> ExecuteVerdict {
    if let Some(line) = state.reservation() {
        if line.overlaps_range(ea, size as u64) {
            state.set_reservation(None);
        }
    }
    if store_buf.insert(ea, size, value as u128) {
        ExecuteVerdict::Continue
    } else {
        ExecuteVerdict::BufferFull
    }
}
