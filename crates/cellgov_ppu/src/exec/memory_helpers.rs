//! Shared load / store helpers used by the per-form execute arms.
//!
//! `load_ze` / `load_se` overlay buffered stores onto the region view
//! so multi-store stitching (eight `stb`s read as one `ld`) and partial
//! overlaps with pre-block memory both resolve correctly.

use crate::exec::verdict::ExecuteVerdict;
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

    /// The fault for a read at `ea` that no view holds whole.
    ///
    /// Its address is the first byte no view holds: the end of the view
    /// the read starts in, when no view holds that byte. Otherwise `ea`:
    /// the read crosses from one view into another, which no single
    /// view can serve.
    #[inline]
    fn refusal(&self, ea: u64) -> cellgov_mem::MemError {
        let holds = |addr: u64| {
            self.views
                .iter()
                .find(|v| v.base <= addr && addr < v.base + v.bytes.len() as u64)
        };
        let first_unheld = holds(ea)
            .map(|v| v.base + v.bytes.len() as u64)
            .filter(|&end| holds(end).is_none())
            .unwrap_or(ea);
        unmapped(first_unheld)
    }
}

/// Synthesize a `MemError::Unmapped` for `ea` with no nearest-region
/// labels populated; the helper does not have a `GuestMemory`
/// reference to walk, only the flat region-view slice.
#[inline]
pub(crate) fn unmapped(ea: u64) -> cellgov_mem::MemError {
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
        return Err(port.refusal(ea));
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
        return Err(port.refusal(ea));
    }
    port.overlay(ea, &mut bytes[..n]);
    Ok(match width {
        Width::B1 => (bytes[0] as i8) as i64 as u64,
        Width::B2 => i16::from_be_bytes([bytes[0], bytes[1]]) as i64 as u64,
        Width::B4 => i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i64 as u64,
        Width::B8 => u64::from_be_bytes(bytes),
    })
}

/// Refuse a cache-block instruction whose byte at `ea` no view holds.
///
/// The check reads no byte and records no read, so the instruction
/// leaves no dependency footprint. The error is the one a one-byte
/// load at `ea` gives.
#[inline]
pub(crate) fn check_block_translates(
    views: &[RegionView<'_>],
    ea: u64,
) -> Result<(), cellgov_mem::MemError> {
    let held = views
        .iter()
        .any(|v| v.base <= ea && ea < v.base + v.bytes.len() as u64);
    if held {
        Ok(())
    } else {
        Err(unmapped(ea))
    }
}

/// Whether the view that holds `ea` is caching-inhibited storage. An
/// address that no view holds returns `false`.
#[inline]
pub(crate) fn caching_inhibited(views: &[RegionView<'_>], ea: u64) -> bool {
    views
        .iter()
        .find(|view| ea >= view.base && ea - view.base < view.bytes.len() as u64)
        .is_some_and(|view| view.caching_inhibited)
}

/// Refuse a store of `len` bytes at `ea` that no one view holds, or
/// that reaches a view whose region refuses writes.
///
/// The error's address is the first refused byte: `ea`, or the end of
/// the writable view the store starts in. That byte lies in the first
/// doubleword the store accesses in the refusing page, which is where
/// the architecture puts the DAR [PPC-Book3 p:67 s:5.5.3]. For a store
/// that runs from one writable view into another, the error names `ea`:
/// no single view can take it.
#[inline]
pub(crate) fn check_store(
    views: &[RegionView<'_>],
    ea: u64,
    len: u64,
) -> Result<(), cellgov_mem::MemError> {
    let end = ea.checked_add(len).ok_or_else(|| unmapped(ea))?;
    for view in views {
        let view_end = view.base + view.bytes.len() as u64;
        if ea < view.base || ea >= view_end {
            continue;
        }
        if let Some(region) = view.write_refused_by {
            return Err(cellgov_mem::MemError::ReservedWrite { addr: ea, region });
        }
        if end <= view_end {
            return Ok(());
        }
        // The rest lies past this view, in a region that refuses it,
        // in no region, or in another writable view.
        return Err(match check_store(views, view_end, end - view_end) {
            Err(e) => e,
            Ok(()) => unmapped(ea),
        });
    }
    Err(unmapped(ea))
}

/// Stage a store. The unit's own reservation survives it: only a
/// store from another processor or mechanism to the granule clears
/// a reservation, so `lwarx; stw <neighbour>; stwcx.` succeeds.
///
/// A store that [`check_store`] refuses faults here, and nothing goes
/// to the buffer.
///
/// [PPC-Book2 p:10 s:1.7.3.1] a reservation is lost to another processor's store or dcbz to the granule, not to the holder's own stores.
#[inline]
pub(crate) fn buffer_store(
    store_buf: &mut StoreBuffer,
    views: &[RegionView<'_>],
    ea: u64,
    size: u8,
    value: u64,
) -> ExecuteVerdict {
    if let Err(e) = check_store(views, ea, u64::from(size)) {
        return ExecuteVerdict::StoreFault(e);
    }
    match store_buf.insert(ea, size, value as u128) {
        Ok(()) => ExecuteVerdict::Continue,
        Err(StoreRefusal::Full) => ExecuteVerdict::BufferFull,
        Err(StoreRefusal::AddressWraps { .. }) => ExecuteVerdict::StoreFault(unmapped(ea)),
    }
}

/// Stage a successful `stwcx.` / `stdcx.`; the caller first passes it
/// through [`check_store`].
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
        Err(StoreRefusal::AddressWraps { .. }) => ExecuteVerdict::StoreFault(unmapped(ea)),
    }
}
