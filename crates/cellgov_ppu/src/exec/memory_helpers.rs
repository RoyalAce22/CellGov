//! Shared load / store helpers used by the per-form execute arms.
//!
//! `load_ze` / `load_se` overlay buffered stores onto the region view
//! so multi-store stitching (eight `stb`s read as one `ld`) and partial
//! overlaps with pre-block memory both resolve correctly.
//!
//! An access of `len` bytes at `ea` covers the bytes `ea`, `ea + 1`, and
//! so on, modulo 2^64. A view of a translating thread can end at 2^64, so
//! the helpers measure a view from its base and never form its end.
//!
//! [PPC-Book1 p:15 s:1.12.2] In 64-bit mode the effective address arithmetic wraps from 2^64 - 1 to 0.

use crate::exec::verdict::ExecuteVerdict;
use crate::exec::PpuFault;
use crate::store_buffer::{StoreBuffer, StoreRefusal};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, RegionView};

/// The number of bytes `view` holds from `addr` to its end, or `None`
/// when `view` does not hold `addr`.
///
/// A view holds no byte at real address 2^64 - 1: no [`ByteRange`]
/// holds that byte, so no effect can name it. Each run of held bytes
/// thus has a [`ByteRange`] at its real address. A region ends
/// below 2^64, so only a view a caller builds over its own addresses,
/// such as a captured memory window, reaches that byte.
#[inline]
fn held_from(view: &RegionView<'_>, addr: u64) -> Option<u64> {
    let len = view.bytes.len() as u64;
    let off = addr.checked_sub(view.base).filter(|&off| off < len)?;
    let real = view.real.checked_add(off).filter(|&r| r != u64::MAX)?;
    Some((len - off).min(u64::MAX - real))
}

/// The position of the first view that holds all `len` bytes at `ea`.
#[inline]
pub(crate) fn view_holding_all(views: &[RegionView<'_>], ea: u64, len: u64) -> Option<usize> {
    views
        .iter()
        .position(|v| held_from(v, ea).is_some_and(|rest| rest >= len))
}

/// Linear search for the `len` bytes at `ea` in one region view.
///
/// O(n) over `region_views`; n is small (single-digit) per dispatch.
/// A hit on a provisional view is logged with that view's memory.
#[inline]
fn load_slice<'v, 'a>(
    region_views: &'v [RegionView<'a>],
    ea: u64,
    len: usize,
) -> Option<(&'v RegionView<'a>, &'a [u8])> {
    let view = &region_views[view_holding_all(region_views, ea, len as u64)?];
    view.note_read(ea, len as u32);
    let offset = (ea - view.base) as usize;
    Some((view, &view.bytes[offset..offset + len]))
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
/// included, at the real addresses its views give. The read intent
/// over-approximates what the load took from committed memory.
/// [`Self::read_committed`] gives the number of read intents one read
/// records.
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
    /// The range may run across views that sit end to end, as a load runs
    /// across adjacent pages, and it wraps from 2^64 - 1 to 0. Returns
    /// `false` when a byte of the range lies in no view; the port then
    /// leaves `out` unchanged and records nothing.
    ///
    /// The read records one read intent at real addresses when each view
    /// it reads maps its bytes to their own addresses. Otherwise it
    /// records one read intent for each view it reads.
    #[inline]
    pub(crate) fn read_committed(&mut self, ea: u64, out: &mut [u8]) -> bool {
        if let Some((view, slice)) = load_slice(self.views, ea, out.len()) {
            out.copy_from_slice(slice);
            self.record_read(view.real_of(ea), out.len() as u64);
            return true;
        }
        if !copy_across_views(self.views, ea, out) {
            return false;
        }
        let len = out.len() as u64;
        let pieces = || pieces_held(self.views, ea, len);
        // A view that maps its bytes to their own addresses never holds the
        // last byte of the space ([`held_from`]), so the whole range is one
        // real range.
        if pieces().all(|(view, _, _)| view.real == view.base) {
            self.record_read(ea, len);
        } else {
            for (view, at, n) in pieces() {
                self.effects.push(Effect::SharedReadIntent {
                    range: real_range(view.real_of(at), n),
                    source: self.source,
                });
            }
        }
        true
    }

    #[inline]
    fn record_read(&mut self, real: u64, len: u64) {
        self.effects.push(Effect::SharedReadIntent {
            range: real_range(real, len),
            source: self.source,
        });
    }

    /// The fault for a read of `len` bytes at `ea` that the views do not
    /// hold.
    ///
    /// Its address is the first byte of the read that no view holds.
    #[inline]
    pub(crate) fn refusal(&self, ea: u64, len: u64) -> cellgov_mem::MemError {
        unmapped(first_unheld(self.views, ea, len).unwrap_or(ea))
    }

    /// See [`caching_inhibited`].
    #[inline]
    pub(crate) fn inhibited(&self, ea: u64, len: u64) -> bool {
        caching_inhibited(self.views, ea, len)
    }

    /// See [`unaligned_inhibited`].
    #[inline]
    pub(crate) fn unaligned_inhibited(&self, ea: u64, width: Width) -> bool {
        unaligned_inhibited(self.views, ea, width.bytes())
    }

    /// See [`inhibited_load`].
    #[inline]
    pub(crate) fn inhibited_load(&self, ea: u64, len: u64) -> ExecuteVerdict {
        inhibited_load(self.views, ea, ea, len)
    }
}

/// The range of the `len` real bytes at `real`.
///
/// # Panics
///
/// Panics if the range ends past 2^64 - 1. Every caller passes a run of
/// bytes that [`held_from`] holds. That function holds no byte at real
/// address 2^64 - 1, so the run ends at 2^64 - 1 at most.
#[inline]
fn real_range(real: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(real), len)
        .expect("held_from holds no byte at real address 2^64 - 1")
}

#[inline]
fn view_holding<'v, 'a>(views: &'v [RegionView<'a>], addr: u64) -> Option<&'v RegionView<'a>> {
    views.iter().find(|v| held_from(v, addr).is_some())
}

/// The real address that the view holding `ea` maps it to, or `ea` when
/// no view holds it.
///
/// Only a translating thread's views map an address to another one.
#[inline]
pub(crate) fn real_address(views: &[RegionView<'_>], ea: u64) -> u64 {
    view_holding(views, ea).map_or(ea, |view| view.real_of(ea))
}

/// The first of the `len` bytes at `ea` that no view holds, or `None`
/// when views that sit end to end hold all of them.
#[inline]
fn first_unheld(views: &[RegionView<'_>], ea: u64, len: u64) -> Option<u64> {
    let held: u64 = pieces_held(views, ea, len).map(|(_, _, n)| n).sum();
    (held < len).then(|| ea.wrapping_add(held))
}

/// Copy the `out.len()` bytes at `ea` from views that sit end to end.
///
/// Each provisional piece logs its read with its view. Returns `false`
/// when a byte of the range lies in no view; the copy then writes and
/// logs nothing.
fn copy_across_views(views: &[RegionView<'_>], ea: u64, out: &mut [u8]) -> bool {
    let len = out.len() as u64;
    if out.is_empty() || first_unheld(views, ea, len).is_some() {
        return false;
    }
    let mut done = 0;
    for (view, at, n) in pieces_held(views, ea, len) {
        let (offset, n) = ((at - view.base) as usize, n as usize);
        view.note_read(at, n as u32);
        out[done..done + n].copy_from_slice(&view.bytes[offset..offset + n]);
        done += n;
    }
    true
}

/// Each view that holds a piece of the `len` bytes at `ea`, with the
/// piece's first byte and length, in the order of the access. The bytes
/// wrap from 2^64 - 1 to 0. The iterator stops at the first byte that no
/// view holds.
///
/// Each piece holds at least one byte, so the iterator yields at most
/// `len` pieces.
fn pieces_held<'v, 'a>(
    views: &'v [RegionView<'a>],
    ea: u64,
    len: u64,
) -> impl Iterator<Item = (&'v RegionView<'a>, u64, u64)> {
    let mut done = 0;
    std::iter::from_fn(move || {
        if done >= len {
            return None;
        }
        let at = ea.wrapping_add(done);
        let (view, rest) = views
            .iter()
            .find_map(|v| held_from(v, at).map(|rest| (v, rest)))?;
        let n = rest.min(len - done);
        done += n;
        Some((view, at, n))
    })
}

/// The one or two spans of the `len` bytes at `ea`, each as its first and
/// last byte. A range that wraps from 2^64 - 1 to 0 has two spans; an
/// empty range has none.
#[inline]
fn spans(ea: u64, len: u64) -> impl Iterator<Item = (u64, u64)> {
    let last = ea.wrapping_add(len.wrapping_sub(1));
    let pair = match len {
        0 => [None, None],
        _ if last >= ea => [Some((ea, last)), None],
        _ => [Some((ea, u64::MAX)), Some((0, last))],
    };
    pair.into_iter().flatten()
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
        return Err(port.refusal(ea, u64::from(size)));
    }
    port.overlay(ea, &mut bytes[..n]);
    Ok(match width {
        Width::B1 => bytes[0] as u64,
        Width::B2 => u16::from_be_bytes([bytes[0], bytes[1]]) as u64,
        Width::B4 => u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as u64,
        Width::B8 => u64::from_be_bytes(bytes),
    })
}

/// [`load_ze`] for a fused pair's load half, after the check of
/// [`unaligned_inhibited`] and the verdict of [`inhibited_load`].
#[inline]
pub(crate) fn load_ze_checked(
    port: &mut LoadPort<'_, '_>,
    ea: u64,
    width: Width,
) -> Result<u64, ExecuteVerdict> {
    if port.unaligned_inhibited(ea, width) {
        return Err(port.inhibited_load(ea, u64::from(width.bytes())));
    }
    load_ze(port, ea, width).map_err(ExecuteVerdict::MemFault)
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
        return Err(port.refusal(ea, u64::from(size)));
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
    if view_holding(views, ea).is_some() {
        Ok(())
    } else {
        Err(unmapped(ea))
    }
}

/// The fault a load of the `len` bytes at `ea` takes on its first byte
/// that no view holds, or `None` when the views hold every byte.
///
/// The check reads no byte and records no read.
#[inline]
pub(crate) fn load_refusal(
    views: &[RegionView<'_>],
    ea: u64,
    len: u64,
) -> Option<cellgov_mem::MemError> {
    first_unheld(views, ea, len).map(unmapped)
}

/// Whether a byte of the `len` bytes at `ea` lies in a caching-inhibited
/// view. A byte that no view holds is not caching-inhibited, and an
/// empty range holds no byte.
#[inline]
pub(crate) fn caching_inhibited(views: &[RegionView<'_>], ea: u64, len: u64) -> bool {
    spans(ea, len).any(|(first, last)| {
        views.iter().any(|view| {
            view.caching_inhibited
                && !view.bytes.is_empty()
                && view.base <= last
                && (first <= view.base || first - view.base < view.bytes.len() as u64)
        })
    })
}

/// Whether an access of `size` bytes at `ea` is unaligned and in
/// caching-inhibited storage.
///
/// [CBE-Handbook p:253 s:9.5.8] The PPE takes the alignment interrupt for a load or store whose operand is unaligned and in caching-inhibited storage.
#[inline]
fn unaligned_inhibited(views: &[RegionView<'_>], ea: u64, size: u8) -> bool {
    ea & u64::from(size - 1) != 0 && caching_inhibited(views, ea, u64::from(size))
}

/// The verdict of a load of the `len` bytes at `start` that the PPE refuses
/// in caching-inhibited storage, with `dar` the instruction's EA.
///
/// A load that runs into a byte no view holds faults on the first such
/// byte.
///
/// [CBE-Handbook p:290 s:9.10] A load's page-fault data storage interrupt precedes its alignment interrupt in caching-inhibited storage.
#[inline]
pub(crate) fn inhibited_load(
    views: &[RegionView<'_>],
    dar: u64,
    start: u64,
    len: u64,
) -> ExecuteVerdict {
    match first_unheld(views, start, len) {
        Some(unheld) => ExecuteVerdict::MemFault(unmapped(unheld)),
        None => ExecuteVerdict::Fault(PpuFault::alignment(dar, false)),
    }
}

/// The verdict of a store of the `len` bytes at `start` that the PPE refuses
/// in caching-inhibited storage, with `dar` the instruction's EA.
///
/// A store that a region also refuses faults as that refusal. The one
/// exception is a view with `store_alignment_first` set: a translating
/// thread's page that refuses stores, whose page table entry has C = 1.
/// There, the store faults on a byte that no view holds, at any place in
/// the range, and otherwise takes the alignment interrupt.
///
/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage follows the page-protection data storage interrupt when C is 0, and precedes it when C is 1.
/// [CBE-Handbook p:290 s:9.10] The page-fault data storage interrupt precedes both.
#[inline]
pub(crate) fn inhibited_store(
    views: &[RegionView<'_>],
    dar: u64,
    start: u64,
    len: u64,
) -> ExecuteVerdict {
    let alignment_first = |addr| view_holding(views, addr).is_some_and(|v| v.store_alignment_first);
    match write_refusal(views, start, len) {
        Some(cellgov_mem::MemError::ReservedWrite { addr, .. }) if alignment_first(addr) => {
            match load_refusal(views, start, len) {
                Some(e) => ExecuteVerdict::StoreFault(e),
                None => ExecuteVerdict::Fault(PpuFault::alignment(dar, true)),
            }
        }
        Some(e) => ExecuteVerdict::StoreFault(e),
        None => ExecuteVerdict::Fault(PpuFault::alignment(dar, true)),
    }
}

/// The refusal of the first of the `len` bytes at `start` that no view
/// holds or whose region refuses writes, or `None` when each byte is in
/// a writable view. The range may span several views, and it wraps from
/// 2^64 - 1 to 0.
#[inline]
fn write_refusal(views: &[RegionView<'_>], start: u64, len: u64) -> Option<cellgov_mem::MemError> {
    let mut done = 0;
    // Each pass moves `done` past the bytes the view at `at` holds, at
    // least one byte, so the loop runs at most `len` times.
    while done < len {
        let at = start.wrapping_add(done);
        let Some((view, rest)) = views
            .iter()
            .find_map(|v| held_from(v, at).map(|rest| (v, rest)))
        else {
            return Some(unmapped(at));
        };
        if let Some(region) = view.write_refused_by {
            return Some(cellgov_mem::MemError::ReservedWrite { addr: at, region });
        }
        done += rest.min(len - done);
    }
    None
}

/// Refuse a store of `len` bytes at `ea` that [`write_refusal`] refuses.
///
/// The error's address is the first refused byte: `ea`, or the end of
/// the writable views the store starts in. That byte lies in the first
/// doubleword the store accesses in the refusing page, which is where
/// the architecture puts the DAR [PPC-Book3 p:67 s:5.5.3].
#[inline]
pub(crate) fn check_store(
    views: &[RegionView<'_>],
    ea: u64,
    len: u64,
) -> Result<(), cellgov_mem::MemError> {
    match write_refusal(views, ea, len) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Stage a store. The unit's own reservation survives it: only a
/// store from another processor or mechanism to the granule clears
/// a reservation, so `lwarx; stw <neighbour>; stwcx.` succeeds.
///
/// A store that [`check_store`] refuses faults here, and nothing goes
/// to the buffer. So does an unaligned store in caching-inhibited
/// storage, as [`inhibited_store`] orders it.
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
    // An operand that runs from a cacheable view into an inhibited one
    // takes the alignment interrupt. `check_store` does not give that
    // interrupt, so this check runs first.
    if unaligned_inhibited(views, ea, size) {
        return inhibited_store(views, ea, ea, u64::from(size));
    }
    if let Err(e) = check_store(views, ea, u64::from(size)) {
        return ExecuteVerdict::StoreFault(e);
    }
    // The count of bytes from `ea` up to 2^64, or 0 when `ea` is 0.
    let top = 0u64.wrapping_sub(ea);
    let staged = if top != 0 && top < u64::from(size) {
        // A store that wraps from 2^64 - 1 to 0 stages as two entries, the
        // first ending at 2^64, so neither entry wraps.
        if !store_buf.has_capacity_for(2) {
            return ExecuteVerdict::BufferFull;
        }
        let low = size - top as u8;
        let low_mask = (1u64 << (8 * u32::from(low))) - 1;
        store_buf
            .stage_effective(
                ea,
                top as u8,
                u128::from(value >> (8 * u32::from(low))),
                None,
            )
            .and_then(|()| store_buf.stage_effective(0, low, u128::from(value & low_mask), None))
    } else {
        store_buf.stage_effective(ea, size, u128::from(value), None)
    };
    staged_verdict(staged, ea)
}

/// The verdict of `staged`, the answer of the buffer to a store at `ea`.
#[inline]
fn staged_verdict(staged: Result<(), StoreRefusal>, ea: u64) -> ExecuteVerdict {
    match staged {
        Ok(()) => ExecuteVerdict::Continue,
        Err(StoreRefusal::Full) => ExecuteVerdict::BufferFull,
        Err(StoreRefusal::AddressWraps { .. }) => ExecuteVerdict::StoreFault(unmapped(ea)),
    }
}

/// Stage a successful `stwcx.` / `stdcx.`; the caller first passes it
/// through [`check_store`].
///
/// The operand is aligned, so it does not wrap from 2^64 - 1 to 0.
#[inline]
pub(crate) fn buffer_conditional_store(
    store_buf: &mut StoreBuffer,
    ea: u64,
    size: u8,
    value: u64,
    emit_at: usize,
) -> ExecuteVerdict {
    staged_verdict(
        store_buf.stage_effective(ea, size, u128::from(value), Some(emit_at)),
        ea,
    )
}
