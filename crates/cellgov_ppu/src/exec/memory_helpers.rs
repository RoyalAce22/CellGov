//! Shared load / store helpers used by the per-form execute arms.
//!
//! `load_ze` / `load_se` overlay buffered stores onto the region view
//! so multi-store stitching (eight `stb`s read as one `ld`) and partial
//! overlaps with pre-block memory both resolve correctly.

use crate::exec::verdict::ExecuteVerdict;
use crate::state::PpuState;
use crate::store_buffer::{StoreBuffer, StoreRefusal};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, RegionView};

/// Linear search for `[ea, ea+len)` covered by one region view.
///
/// O(n) over `region_views`; n is small (single-digit) per dispatch.
/// A hit on a provisional view is logged with that view's memory.
#[inline]
fn load_slice<'a>(region_views: &[RegionView<'a>], ea: u64, len: usize) -> Option<&'a [u8]> {
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

/// What one block's loads read, and where they record each read.
///
/// [`Self::read_committed`] is the one point at which a PPU load
/// reaches committed memory, so it is also the one point that emits
/// `Effect::SharedReadIntent`. Two classes of load emit nothing:
///
/// - a load the store buffer forwards whole, which observes this
///   unit's own uncommitted store instead of committed memory;
/// - a load that faults, since a fault discards the whole step.
///
/// Every other load records its whole range, the overlaid bytes
/// included. The read intent over-approximates what the load took
/// from committed memory.
pub(crate) struct LoadPort<'v, 'e> {
    views: &'v [RegionView<'v>],
    store_buf: &'v StoreBuffer,
    effects: &'e mut Vec<Effect>,
    source: UnitId,
}

impl<'v, 'e> LoadPort<'v, 'e> {
    #[inline]
    pub(crate) fn new(
        views: &'v [RegionView<'v>],
        store_buf: &'v StoreBuffer,
        effects: &'e mut Vec<Effect>,
        source: UnitId,
    ) -> Self {
        Self {
            views,
            store_buf,
            effects,
            source,
        }
    }

    /// See [`StoreBuffer::forward`].
    #[inline]
    pub(crate) fn forward(&self, ea: u64, len: u8) -> Option<u128> {
        self.store_buf.forward(ea, len)
    }

    /// See [`StoreBuffer::overlay_range`].
    #[inline]
    pub(crate) fn overlay(&self, base: u64, out: &mut [u8]) {
        self.store_buf.overlay_range(base, out);
    }

    /// Copy the committed bytes at `ea` into `out` and record the read.
    ///
    /// Returns `false` when no region view covers the range; the port
    /// then leaves `out` unchanged and records nothing.
    #[inline]
    pub(crate) fn read_committed(&mut self, ea: u64, out: &mut [u8]) -> bool {
        let Some(slice) = load_slice(self.views, ea, out.len()) else {
            return false;
        };
        out.copy_from_slice(slice);
        let range = ByteRange::new(GuestAddr::new(ea), out.len() as u64)
            .expect("load_slice rejects an (ea, len) that overflows u64");
        self.effects.push(Effect::SharedReadIntent {
            range,
            source: self.source,
        });
        true
    }
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

/// The widths of a scalar load or store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Width {
    /// One byte.
    B1,
    /// Halfword.
    B2,
    /// Word.
    B4,
    /// Doubleword.
    B8,
}

impl Width {
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
    port: &mut LoadPort<'_, '_>,
    ea: u64,
    width: Width,
) -> Result<u64, cellgov_mem::MemError> {
    let size = width.bytes();
    if let Some(val) = port.forward(ea, size) {
        return Ok(val as u64);
    }
    let mut bytes = [0u8; 8];
    let n = size as usize;
    if !port.read_committed(ea, &mut bytes[..n]) {
        return Err(unmapped(ea));
    }
    port.overlay(ea, &mut bytes[..n]);
    Ok(match width {
        Width::B1 => bytes[0] as u64,
        Width::B2 => u16::from_be_bytes([bytes[0], bytes[1]]) as u64,
        Width::B4 => u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as u64,
        Width::B8 => u64::from_be_bytes(bytes),
    })
}

/// Sign-extending load with store-buffer forwarding. See [`load_ze`].
#[inline]
pub(crate) fn load_se(
    port: &mut LoadPort<'_, '_>,
    ea: u64,
    width: Width,
) -> Result<u64, cellgov_mem::MemError> {
    let size = width.bytes();
    if let Some(val) = port.forward(ea, size) {
        // `forward` right-aligns `size` bytes; sign must come from
        // the size's MSB, not u64 bit 63 (always 0 for sub-doubleword).
        return Ok(match width {
            Width::B1 => (val as u8 as i8) as i64 as u64,
            Width::B2 => (val as u16 as i16) as i64 as u64,
            Width::B4 => (val as u32 as i32) as i64 as u64,
            Width::B8 => val as u64,
        });
    }
    let mut bytes = [0u8; 8];
    let n = size as usize;
    if !port.read_committed(ea, &mut bytes[..n]) {
        return Err(unmapped(ea));
    }
    port.overlay(ea, &mut bytes[..n]);
    Ok(match width {
        Width::B1 => (bytes[0] as i8) as i64 as u64,
        Width::B2 => i16::from_be_bytes([bytes[0], bytes[1]]) as i64 as u64,
        Width::B4 => i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i64 as u64,
        Width::B8 => u64::from_be_bytes(bytes),
    })
}

/// Stage a store. The unit's own reservation survives it: only a
/// store from another processor or mechanism to the granule clears
/// a reservation, so `lwarx; stw <neighbour>; stwcx.` succeeds.
///
/// A store whose last byte lies past the end of the address space
/// faults as unmapped: no region can hold it.
// [PPC-Book2 p:10 s:1.7.3.1] a reservation is lost to another processor's store or dcbz to the granule, not to the holder's own stores.
#[inline]
pub(crate) fn buffer_store(
    store_buf: &mut StoreBuffer,
    _state: &mut PpuState,
    ea: u64,
    size: u8,
    value: u64,
) -> ExecuteVerdict {
    match store_buf.insert(ea, size, value as u128) {
        Ok(()) => ExecuteVerdict::Continue,
        Err(StoreRefusal::Full) => ExecuteVerdict::BufferFull,
        Err(StoreRefusal::AddressWraps { .. }) => ExecuteVerdict::MemFault(unmapped(ea)),
    }
}

/// Stage a successful `stwcx.` / `stdcx.`; see [`buffer_store`] for
/// the wrap refusal.
#[inline]
pub(crate) fn buffer_conditional_store(
    store_buf: &mut StoreBuffer,
    ea: u64,
    size: u8,
    value: u64,
    emit_at: usize,
) -> ExecuteVerdict {
    match store_buf.insert_conditional(ea, size, value as u128, emit_at) {
        Ok(()) => ExecuteVerdict::Continue,
        Err(StoreRefusal::Full) => ExecuteVerdict::BufferFull,
        Err(StoreRefusal::AddressWraps { .. }) => ExecuteVerdict::MemFault(unmapped(ea)),
    }
}
