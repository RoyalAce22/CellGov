//! Intra-block store-forwarding buffer.
//!
//! Forwarding contract: a load that is fully covered by an earlier
//! store in the same block observes that store's bytes; committed
//! memory still holds the pre-block state until [`StoreBuffer::flush`]
//! emits `Effect::SharedWriteIntent` packets at the block boundary.
//
// [PPC-Book2 p:8 s:1.7 Shared Storage] weakly consistent storage model: stores need not be globally visible in program order, only as observed by the executing processor.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_time::GuestTicks;

const CAPACITY: usize = 64;

/// A single pending store entry.
///
/// `emit_at == Some(n)` marks a successful `stwcx`/`stdcx`, with `n`
/// the effect-vector length when it executed. [`StoreBuffer::flush`]
/// emits it as `ConditionalStore` at that slot, ahead of any
/// `ReservationAcquire` or `SharedReadIntent` pushed straight into the
/// vector afterwards:
/// the commit pipeline drops the emitter's reservation when it
/// applies a `ConditionalStore`, so an acquire that committed first
/// would be lost before its own `stwcx` used it.
///
/// `len` is the architectural store width in bytes and must satisfy
/// `1 <= len <= 16`. The lower bound rules out zero-byte stores
/// (no PPC store instruction has zero width); the upper bound is the
/// `value: u128` payload's capacity (`stvx`/`dcbz`-by-granule peak at
/// 16 bytes per entry).
///
/// [PPC-Book2 p:10 s:1.7.3.1 Reservations] a later lwarx/ldarx clears the earlier reservation and establishes a new one; a stwcx./stdcx. clears only the reservation that precedes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StoreEntry {
    addr: u64,
    len: u8,
    emit_at: Option<usize>,
    value: u128,
}

/// One pending store at the current PPU block boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingStore {
    /// Guest address of the first byte.
    pub addr: u64,
    /// Architectural store width in bytes.
    pub len: u8,
    /// Effect-list position that a conditional store reserves.
    pub conditional_effect_index: Option<usize>,
    /// Big-endian value in the forwarding buffer.
    pub value: u128,
}

/// Why the buffer refused a store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StoreRefusal {
    /// The buffer holds its capacity of entries.
    #[error("store buffer is full")]
    Full,
    /// The store runs past the address space.
    ///
    /// [`StoreBuffer::insert`] and [`StoreBuffer::insert_conditional`]
    /// also refuse a store whose last byte is the last byte of the
    /// address space, because no [`ByteRange`] holds that byte.
    #[error("store at 0x{addr:016x} of {len} bytes wraps the address space")]
    AddressWraps {
        /// First byte of the store.
        addr: u64,
        /// Store width in bytes.
        len: u8,
    },
}

/// The pieces of a buffered store at the addresses its effects carry:
/// each piece's address, its offset into the store, and its length.
pub(crate) type StorePieces = Vec<(u64, u64, u64)>;

impl StoreEntry {
    /// The last byte the entry writes.
    ///
    /// An entry ends at 2^64 at most, so the last byte is an address. A
    /// zero-width entry, which only a release build can stage, ends one
    /// byte below its start. At address 0 that wraps to 2^64 - 1, so
    /// [`StoreBuffer::forward`] and [`StoreBuffer::overlay_range`] skip a
    /// zero-width entry by its width.
    fn last(&self) -> u64 {
        self.addr.wrapping_add(u64::from(self.len)).wrapping_sub(1)
    }

    /// Push the entry's effects onto `out`, one for each piece `pieces`
    /// gives.
    ///
    /// # Panics
    ///
    /// Panics if a piece ends past 2^64 - 1. A thread that does not
    /// translate stages no entry that ends at 2^64, and a real piece lies
    /// below 2^62.
    fn push_effects(
        &self,
        source: UnitId,
        pieces: &impl Fn(u64, u8) -> StorePieces,
        out: &mut Vec<Effect>,
    ) {
        let bytes = &self.value.to_be_bytes()[16 - self.len as usize..];
        let range = |at: u64, n: u64| {
            ByteRange::new(GuestAddr::new(at), n)
                .expect("an effect's store piece ends below 2^64; only a translating thread stages an entry that ends at 2^64")
        };
        let parts = pieces(self.addr, self.len);
        if self.emit_at.is_some() {
            // An aligned conditional store lies in one page.
            debug_assert_eq!(parts.len(), 1, "a conditional store across pages");
            out.push(Effect::ConditionalStore {
                range: range(parts[0].0, u64::from(self.len)),
                bytes: WritePayload::from_slice(bytes),
                source,
                source_time: GuestTicks::ZERO,
            });
            return;
        }
        for (at, off, n) in parts {
            out.push(Effect::shared_write(
                range(at, n),
                WritePayload::from_slice(&bytes[off as usize..(off + n) as usize]),
                source,
                GuestTicks::ZERO,
            ));
        }
    }
}

/// The one piece of a store at its own address.
fn same_address(addr: u64, len: u8) -> StorePieces {
    vec![(addr, 0, u64::from(len))]
}

/// Fixed-capacity (64-entry) store-forwarding buffer.
///
/// [`Self::insert`] appends stores in program order; [`Self::forward`]
/// scans in reverse, so the most recent store that touches a load
/// decides what the load sees.
#[derive(Clone)]
pub struct StoreBuffer {
    entries: Vec<StoreEntry>,
}

impl Default for StoreBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl StoreBuffer {
    /// Create an empty store buffer.
    #[inline]
    pub fn new() -> Self {
        Self {
            entries: Vec::with_capacity(CAPACITY),
        }
    }

    /// Number of pending stores.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the buffer is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The lowest start and highest end of the pending stores, or `None`
    /// when the buffer is empty.
    ///
    /// The end of an entry that ends at 2^64 saturates to 2^64 - 1. Only
    /// a translating thread stages such an entry, and the batch loop
    /// compares no translating thread's span with an address.
    pub fn span(&self) -> Option<(u64, u64)> {
        self.entries.iter().fold(None, |acc, e| {
            let end = e.addr.saturating_add(u64::from(e.len));
            Some(acc.map_or((e.addr, end), |(lo, hi): (u64, u64)| {
                (lo.min(e.addr), hi.max(end))
            }))
        })
    }

    /// Whether the buffer is full.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.entries.len() >= CAPACITY
    }

    /// Returns a program-order record of pending stores.
    pub fn snapshot(&self) -> Vec<PendingStore> {
        self.entries
            .iter()
            .map(|entry| PendingStore {
                addr: entry.addr,
                len: entry.len,
                conditional_effect_index: entry.emit_at,
                value: entry.value,
            })
            .collect()
    }

    /// The `(addr, len)` of each pending store from position `from` on,
    /// in program order.
    #[inline]
    pub(crate) fn stores_from(&self, from: usize) -> impl Iterator<Item = (u64, u8)> + '_ {
        self.entries
            .get(from..)
            .unwrap_or_default()
            .iter()
            .map(|e| (e.addr, e.len))
    }

    /// [`Self::stores_from`] with a third field, the entry's value.
    ///
    /// The stored bytes are the low `len` bytes of the value. The bits
    /// above them can hold register bits, so the caller masks them.
    pub(crate) fn values_from(&self, from: usize) -> impl Iterator<Item = (u64, u8, u128)> + '_ {
        self.entries
            .get(from..)
            .unwrap_or_default()
            .iter()
            .map(|e| (e.addr, e.len, e.value))
    }

    /// Whether `n` more entries fit. Multi-store instructions
    /// (`stvx`, `dcbz`) call this before staging any store so a
    /// `BufferFull` mid-instruction does not leave a partial commit
    /// the retry path would duplicate.
    #[inline]
    pub fn has_capacity_for(&self, n: usize) -> bool {
        self.entries.len() + n <= CAPACITY
    }

    /// Clear all entries.
    #[inline]
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Drop every entry from position `len` on.
    #[inline]
    pub(crate) fn truncate(&mut self, len: usize) {
        self.entries.truncate(len);
    }

    /// Insert a pending store.
    ///
    /// # Errors
    ///
    /// - [`StoreRefusal::AddressWraps`]: `addr + len` overflows `u64`, so
    ///   the store reaches the last byte of the address space.
    /// - [`StoreRefusal::Full`]: the buffer holds its capacity. The
    ///   caller flushes (yields the block), then retries.
    ///
    /// # Panics
    ///
    /// Panics in a debug build if `len` is outside `1..=16`.
    #[inline]
    pub fn insert(&mut self, addr: u64, len: u8, value: u128) -> Result<(), StoreRefusal> {
        Self::refuse_end_at_top(addr, len)?;
        self.stage_effective(addr, len, value, None)
    }

    /// Insert a successful `stwcx` / `stdcx`, forwarded to later loads
    /// in the block and emitted by [`Self::flush`] as a
    /// `ConditionalStore` at effect slot `emit_at` (see `StoreEntry`).
    ///
    /// [PPC-Book2 p:9 s:1.7.3 Atomic Update] stwcx./stdcx. commit through the ConditionalStore effect path; the entry keeps their bytes in program order with the block's plain stores.
    ///
    /// # Errors
    ///
    /// Returns the same refusals as [`Self::insert`].
    ///
    /// # Panics
    ///
    /// Panics as [`Self::insert`] does.
    #[inline]
    pub fn insert_conditional(
        &mut self,
        addr: u64,
        len: u8,
        value: u128,
        emit_at: usize,
    ) -> Result<(), StoreRefusal> {
        Self::refuse_end_at_top(addr, len)?;
        self.stage_effective(addr, len, value, Some(emit_at))
    }

    /// Refuse a store whose last byte is the last byte of the address
    /// space or lies past it: [`Self::flush`] gives it no [`ByteRange`].
    #[inline]
    fn refuse_end_at_top(addr: u64, len: u8) -> Result<(), StoreRefusal> {
        match addr.checked_add(u64::from(len)) {
            Some(_) => Ok(()),
            None => Err(StoreRefusal::AddressWraps { addr, len }),
        }
    }

    /// Insert a pending store at an effective address; a `Some` `emit_at`
    /// makes it a conditional store at that effect slot.
    ///
    /// The store may end at 2^64, because a translating thread's views
    /// hold the last byte of the effective space. The caller publishes
    /// such an entry through [`Self::flush_pieces`], which moves it to
    /// real addresses. [`Self::flush`] panics on it.
    ///
    /// # Errors
    ///
    /// - [`StoreRefusal::AddressWraps`]: the store runs past 2^64 - 1.
    /// - [`StoreRefusal::Full`]: the buffer holds its capacity.
    ///
    /// # Panics
    ///
    /// Panics as [`Self::insert`] does.
    #[inline]
    pub(crate) fn stage_effective(
        &mut self,
        addr: u64,
        len: u8,
        value: u128,
        emit_at: Option<usize>,
    ) -> Result<(), StoreRefusal> {
        debug_assert!(
            len > 0 && len <= 16,
            "store width out of range: len={len} (architectural stores are 1..=16 bytes; \
             value: u128 carries at most 16 bytes)"
        );
        if addr.checked_add(u64::from(len).saturating_sub(1)).is_none() {
            return Err(StoreRefusal::AddressWraps { addr, len });
        }
        if self.entries.len() >= CAPACITY {
            return Err(StoreRefusal::Full);
        }
        self.entries.push(StoreEntry {
            addr,
            len,
            emit_at,
            value,
        });
        Ok(())
    }

    /// Forward a load from the most recent buffered store that touches it.
    ///
    /// On `None` the caller reads pre-block memory, then calls
    /// [`Self::overlay_range`]. The result is `None` when:
    ///
    /// - no buffered store touches the load;
    /// - the most recent touching store covers only part of the load,
    ///   even if an older store covers all of it: the later store
    ///   replaced some of the older store's bytes;
    /// - the load runs past the last byte of the address space, so no
    ///   entry holds it whole.
    ///
    /// # Performance
    /// O(n) reverse scan over up to 64 entries per load.
    #[inline]
    pub fn forward(&self, addr: u64, len: u8) -> Option<u128> {
        let load_last = addr.checked_add(u64::from(len).checked_sub(1)?)?;
        for i in (0..self.entries.len()).rev() {
            let e = &self.entries[i];
            if e.len == 0 || e.addr > load_last || e.last() < addr {
                continue;
            }
            if e.addr > addr || e.last() < load_last {
                return None;
            }
            let all = e.value.to_be_bytes();
            let store_off = 16 - e.len as usize;
            let load_off = store_off + (addr - e.addr) as usize;
            let mut out = [0u8; 16];
            let dest = 16 - len as usize;
            out[dest..].copy_from_slice(&all[load_off..load_off + len as usize]);
            return Some(u128::from_be_bytes(out));
        }
        None
    }

    /// Emit pending stores in program order -- `SharedWriteIntent`
    /// for plain entries, `ConditionalStore` for successful
    /// `stwcx`/`stdcx` -- and clear the buffer.
    ///
    /// [PPC-Book2 p:28 s:3.3.3] a memory barrier orders one processor's stores as other processors see them.
    /// The flush runs at the block boundary. The step's commit then applies
    /// the stores as one batch in program order. Thus no barrier inside the
    /// block changes what another unit sees.
    ///
    /// # Panics
    ///
    /// If `effects` was truncated below a recorded `emit_at` since
    /// the conditional store executed. The caller keeps the vector
    /// append-only between execute and flush; a fault clears both
    /// the vector and this buffer together, or truncates both back to
    /// the same instruction boundary.
    pub fn flush(&mut self, effects: &mut Vec<Effect>, source: UnitId) {
        self.flush_pieces(effects, source, same_address);
    }

    /// [`Self::flush`], with each store's effects at the addresses that
    /// `pieces` gives for its address and length, one effect per piece.
    ///
    /// A translating thread passes the real pieces of each store, so no
    /// effect carries an effective address.
    ///
    /// # Panics
    ///
    /// Panics as [`Self::flush`] does, and if a piece ends past
    /// 2^64 - 1.
    pub(crate) fn flush_pieces(
        &mut self,
        effects: &mut Vec<Effect>,
        source: UnitId,
        pieces: impl Fn(u64, u8) -> StorePieces,
    ) {
        let mut next = 0usize;
        let mut inserted = 0usize;
        let mut group = Vec::new();
        for i in 0..self.entries.len() {
            let Some(at) = self.entries[i].emit_at else {
                continue;
            };
            let pos = at + inserted;
            assert!(
                pos <= effects.len(),
                "conditional store recorded effect slot {at} (+{inserted} inserted by this \
                 flush) but the effect vector holds only {} entries; it was truncated between \
                 execute and flush",
                effects.len()
            );
            group.clear();
            for e in &self.entries[next..=i] {
                e.push_effects(source, &pieces, &mut group);
            }
            // `emit_at` counts the effects ahead of the slot before this
            // flush, so each slot moves by the effects inserted ahead of it.
            inserted += group.len();
            if pos == effects.len() {
                effects.append(&mut group);
            } else {
                let tail = effects.split_off(pos);
                effects.append(&mut group);
                effects.extend(tail);
            }
            next = i + 1;
        }
        for e in &self.entries[next..] {
            e.push_effects(source, &pieces, effects);
        }
        self.entries.clear();
    }

    /// Overlay every buffered store onto `output`, where `output`
    /// starts at guest address `base`. Stores are applied in
    /// program order, so a later store's bytes overwrite an earlier
    /// store's bytes in the same overlapping window.
    ///
    /// Used by 16-byte aligned vector loads to merge a region read
    /// with any partial-overlap stores in the buffer; the alternative
    /// (yield + flush + retry on every partial overlap) collapses
    /// batch size when scalar stores and vector loads share a line.
    ///
    /// Conditional entries are included because within a single step
    /// the context is frozen, so committed memory still holds the
    /// pre-batch bytes even after `stwcx`/`stdcx` emits a
    /// `ConditionalStore` effect. The buffered conditional entry is
    /// the only intra-step record of those bytes, and a vector load
    /// that overlaps must observe them.
    pub fn overlay_range(&self, base: u64, output: &mut [u8]) {
        // The window wraps from 2^64 - 1 to 0, as the load that reads it
        // does: it is one or two spans, each at its offset into `output`.
        let len = output.len() as u64;
        let last = base.wrapping_add(len.wrapping_sub(1));
        let spans = match len {
            0 => [None, None],
            _ if last >= base => [Some((base, last, 0)), None],
            _ => [
                Some((base, u64::MAX, 0)),
                Some((0, last, 0u64.wrapping_sub(base))),
            ],
        };
        for e in &self.entries {
            for (first, span_last, at) in spans.into_iter().flatten() {
                let overlap_first = e.addr.max(first);
                let overlap_last = e.last().min(span_last);
                if e.len == 0 || overlap_first > overlap_last {
                    continue;
                }
                let all = e.value.to_be_bytes();
                let store_off = 16 - e.len as usize;
                let in_entry = store_off + (overlap_first - e.addr) as usize;
                let in_output = (at + (overlap_first - first)) as usize;
                let n = (overlap_last - overlap_first) as usize + 1;
                output[in_output..in_output + n].copy_from_slice(&all[in_entry..in_entry + n]);
            }
        }
    }

    /// Whether any pending store overlaps `[shadow_base, shadow_end)`.
    ///
    /// The check includes conditional entries. A successful `stwcx`/`stdcx`
    /// changes committed memory when its `ConditionalStore` effect commits,
    /// as a plain store does. A failed `stwcx`/`stdcx` does not reach
    /// `insert_conditional`.
    #[inline]
    pub fn has_store_in_range(&self, shadow_base: u64, shadow_end: u64) -> bool {
        self.entries
            .iter()
            .any(|e| e.addr < shadow_end && e.last() >= shadow_base)
    }
}

#[cfg(test)]
#[path = "tests/store_buffer_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/store_buffer_refusal_tests.rs"]
mod refusal_tests;

#[cfg(test)]
#[path = "tests/store_buffer_proptests.rs"]
mod proptests;

#[cfg(test)]
#[path = "tests/store_buffer_top_tests.rs"]
mod top_tests;
