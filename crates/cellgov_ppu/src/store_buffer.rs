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
/// `ReservationAcquire` pushed straight into the vector afterwards:
/// the commit pipeline drops the emitter's reservation when it
/// applies a `ConditionalStore`, so an acquire that committed first
/// would be lost before its own `stwcx` used it.
///
/// `len` is the architectural store width in bytes and must satisfy
/// `1 <= len <= 16`. The lower bound rules out zero-byte stores
/// (no PPC store instruction has zero width); the upper bound is the
/// `value: u128` payload's capacity (`stvx`/`dcbz`-by-granule peak at
/// 16 bytes per entry).
// [PPC-Book2 p:10 s:1.7.3.1 Reservations] a later lwarx/ldarx clears the earlier reservation and establishes a new one; a stwcx./stdcx. clears only the reservation that precedes it.
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

impl StoreEntry {
    fn effect(&self, source: UnitId) -> Effect {
        let bytes = &self.value.to_be_bytes();
        let offset = 16 - self.len as usize;
        let payload = WritePayload::from_slice(&bytes[offset..]);
        // The insert paths reject any (addr, len) that would
        // overflow on `addr + len`, so `ByteRange::new` cannot
        // fail here; a `None` would be an invariant breach, and
        // panicking surfaces it rather than silently dropping a
        // guest store.
        let range = ByteRange::new(GuestAddr::new(self.addr), self.len as u64)
            .expect("store buffer entry violates addr+len invariant; insert should reject");
        if self.emit_at.is_some() {
            Effect::ConditionalStore {
                range,
                bytes: payload,
                source,
                source_time: GuestTicks::ZERO,
            }
        } else {
            Effect::shared_write(range, payload, source, GuestTicks::ZERO)
        }
    }
}

/// Fixed-capacity (64-entry) store-forwarding buffer.
///
/// Stores are appended in program order; [`Self::forward`] scans in
/// reverse so the most recent covering store wins. Full buffer
/// returns `false` from `insert`; the caller must yield the block.
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

    /// Insert a pending store. Returns `false` when the buffer is
    /// full; caller must flush (yield the block) before retrying.
    #[inline]
    pub fn insert(&mut self, addr: u64, len: u8, value: u128) -> bool {
        debug_assert!(
            len > 0 && len <= 16,
            "store width out of range: len={len} (architectural stores are 1..=16 bytes; \
             value: u128 carries at most 16 bytes)"
        );
        debug_assert!(
            addr.checked_add(len as u64).is_some(),
            "store addr=0x{addr:x} + len={len} overflows u64"
        );
        if self.entries.len() >= CAPACITY {
            return false;
        }
        self.entries.push(StoreEntry {
            addr,
            len,
            emit_at: None,
            value,
        });
        true
    }

    /// Insert a successful `stwcx` / `stdcx`, forwarded to later loads
    /// in the block and emitted by [`Self::flush`] as a
    /// `ConditionalStore` at effect slot `emit_at` (see `StoreEntry`).
    /// Returns `false` when the buffer is full.
    // [PPC-Book2 p:9 s:1.7.3 Atomic Update] stwcx./stdcx. commit through the ConditionalStore effect path; the entry keeps their bytes in program order with the block's plain stores.
    #[inline]
    pub fn insert_conditional(&mut self, addr: u64, len: u8, value: u128, emit_at: usize) -> bool {
        debug_assert!(
            len > 0 && len <= 16,
            "conditional store width out of range: len={len}"
        );
        debug_assert!(
            addr.checked_add(len as u64).is_some(),
            "conditional store addr=0x{addr:x} + len={len} overflows u64"
        );
        if self.entries.len() >= CAPACITY {
            return false;
        }
        self.entries.push(StoreEntry {
            addr,
            len,
            emit_at: Some(emit_at),
            value,
        });
        true
    }

    /// Fast-path forward: a single buffered store fully covers the
    /// load. Returns `None` for partial overlap or when multiple
    /// narrower stores tile the range -- the caller must read
    /// pre-block memory and call [`Self::overlay_range`] to stitch
    /// in the buffered bytes (see `load_ze` / `load_se` in
    /// `exec.rs` and `read_aligned_16` in `exec/mem.rs`).
    ///
    /// # Performance
    /// O(n) reverse scan over up to 64 entries per load.
    #[inline]
    pub fn forward(&self, addr: u64, len: u8) -> Option<u128> {
        let load_end = addr + len as u64;
        for i in (0..self.entries.len()).rev() {
            let e = &self.entries[i];
            let store_end = e.addr + e.len as u64;
            if e.addr <= addr && store_end >= load_end {
                let all = e.value.to_be_bytes();
                let store_off = 16 - e.len as usize;
                let load_off = store_off + (addr - e.addr) as usize;
                let mut out = [0u8; 16];
                let dest = 16 - len as usize;
                out[dest..].copy_from_slice(&all[load_off..load_off + len as usize]);
                return Some(u128::from_be_bytes(out));
            }
        }
        None
    }

    /// Emit pending stores in program order -- `SharedWriteIntent`
    /// for plain entries, `ConditionalStore` for successful
    /// `stwcx`/`stdcx` -- and clear the buffer.
    ///
    /// # Panics
    ///
    /// If `effects` was truncated below a recorded `emit_at` since
    /// the conditional store executed. The caller keeps the vector
    /// append-only between execute and flush; a fault clears both
    /// the vector and this buffer together.
    // [PPC-Book2 p:28 s:3.3 eieio] block-boundary flush models the memory-barrier ordering of Load/Store accesses with respect to other processors.
    pub fn flush(&mut self, effects: &mut Vec<Effect>, source: UnitId) {
        let mut next = 0usize;
        let mut inserted = 0usize;
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
            let group = self.entries[next..=i].iter().map(|e| e.effect(source));
            if pos == effects.len() {
                effects.extend(group);
            } else {
                let tail = effects.split_off(pos);
                effects.extend(group);
                effects.extend(tail);
            }
            inserted += i + 1 - next;
            next = i + 1;
        }
        effects.extend(self.entries[next..].iter().map(|e| e.effect(source)));
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
        debug_assert!(
            base.checked_add(output.len() as u64).is_some(),
            "overlay base=0x{base:x} + len={} overflows u64",
            output.len()
        );
        let base_end = base + output.len() as u64;
        for e in &self.entries {
            let entry_end = e.addr + e.len as u64;
            let overlap_start = e.addr.max(base);
            let overlap_end = entry_end.min(base_end);
            if overlap_start >= overlap_end {
                continue;
            }
            let all = e.value.to_be_bytes();
            let store_off = 16 - e.len as usize;
            let in_entry = store_off + (overlap_start - e.addr) as usize;
            let in_output = (overlap_start - base) as usize;
            let n = (overlap_end - overlap_start) as usize;
            output[in_output..in_output + n].copy_from_slice(&all[in_entry..in_entry + n]);
        }
    }

    /// Whether any pending store overlaps `[shadow_base, shadow_end)`.
    ///
    /// A `true` result forces an early block yield so the shadow is
    /// re-decoded before fetch observes stale bytes. Conditional
    /// entries are included: a successful `stwcx`/`stdcx` mutates
    /// committed memory once its `ConditionalStore` effect commits,
    /// so a conditional store into the text region must invalidate
    /// the shadow exactly like a plain store. (Failed `stwcx`/`stdcx`
    /// never reach `insert_conditional`.)
    #[inline]
    pub fn has_store_in_range(&self, shadow_base: u64, shadow_end: u64) -> bool {
        for e in &self.entries {
            let store_end = e.addr + e.len as u64;
            if e.addr < shadow_end && store_end > shadow_base {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
#[path = "tests/store_buffer_tests.rs"]
mod tests;
