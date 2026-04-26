//! Intra-block store-forwarding buffer.
//!
//! Forwarding contract: a load that is fully covered by an earlier
//! store in the same block observes that store's bytes; committed
//! memory still holds the pre-block state until [`StoreBuffer::flush`]
//! emits `Effect::SharedWriteIntent` packets at the block boundary.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_time::GuestTicks;

const CAPACITY: usize = 64;

/// A single pending store entry.
///
/// `conditional == true` entries are visible to [`StoreBuffer::forward`]
/// but skipped by [`StoreBuffer::flush`] -- the `ConditionalStore`
/// effect is emitted directly by `stwcx`/`stdcx`, so flushing again
/// would double-commit.
///
/// `len` is the architectural store width in bytes and must satisfy
/// `1 <= len <= 16`. The lower bound rules out zero-byte stores
/// (no PPC store instruction has zero width); the upper bound is the
/// `value: u128` payload's capacity (`stvx`/`dcbz`-by-granule peak at
/// 16 bytes per entry).
#[derive(Clone, Copy)]
struct StoreEntry {
    addr: u64,
    len: u8,
    conditional: bool,
    value: u128,
}

/// Fixed-capacity (64-entry) store-forwarding buffer.
///
/// Stores are appended in program order; [`Self::forward`] scans in
/// reverse so the most recent covering store wins. Full buffer
/// returns `false` from `insert`; the caller must yield the block.
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
            conditional: false,
            value,
        });
        true
    }

    /// Insert a successful `stwcx` / `stdcx` for forwarding only.
    ///
    /// Flush skips this entry (see [`StoreEntry`]). Returns `false`
    /// when the buffer is full.
    #[inline]
    pub fn insert_conditional(&mut self, addr: u64, len: u8, value: u128) -> bool {
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
            conditional: true,
            value,
        });
        true
    }

    /// Forward a load from the buffer, or `None` to fall through to
    /// committed memory.
    ///
    /// Only full-coverage matches forward. Partial overlaps return
    /// `None`: committed memory still holds the pre-block state, so
    /// the fallback read is correct.
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

    /// Emit pending stores as `SharedWriteIntent` effects in program
    /// order and clear the buffer. Conditional entries are skipped.
    pub fn flush(&mut self, effects: &mut Vec<Effect>, source: UnitId) {
        for i in 0..self.entries.len() {
            let e = &self.entries[i];
            if e.conditional {
                continue;
            }
            let bytes = &e.value.to_be_bytes();
            let offset = 16 - e.len as usize;
            let payload = WritePayload::from_slice(&bytes[offset..]);
            // The insert paths reject any (addr, len) that would
            // overflow on `addr + len`, so `ByteRange::new` cannot
            // fail here; treat a `None` as a load-bearing invariant
            // breach rather than silently dropping a guest store.
            let range = ByteRange::new(GuestAddr::new(e.addr), e.len as u64)
                .expect("store buffer entry violates addr+len invariant; insert should reject");
            effects.push(Effect::SharedWriteIntent {
                range,
                bytes: payload,
                ordering: PriorityClass::Normal,
                source,
                source_time: GuestTicks::ZERO,
            });
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
    /// Conditional entries are intentionally included: within a
    /// single step the context is frozen, so committed memory still
    /// holds the pre-batch bytes even after `stwcx`/`stdcx` emits a
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
mod tests {
    use super::*;

    #[test]
    fn empty_buffer() {
        let buf = StoreBuffer::new();
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
        assert!(!buf.is_full());
        assert!(buf.forward(0, 4).is_none());
    }

    #[test]
    fn insert_and_forward_u32() {
        let mut buf = StoreBuffer::new();
        let val = 0xDEADBEEF_u128;
        assert!(buf.insert(0x100, 4, val));
        assert_eq!(buf.len(), 1);

        let fwd = buf.forward(0x100, 4);
        assert_eq!(fwd, Some(0xDEADBEEF));
    }

    #[test]
    fn insert_and_forward_u8() {
        let mut buf = StoreBuffer::new();
        assert!(buf.insert(0x200, 1, 0x42));
        let fwd = buf.forward(0x200, 1);
        assert_eq!(fwd, Some(0x42));
    }

    #[test]
    fn insert_and_forward_u64() {
        let mut buf = StoreBuffer::new();
        let val = 0xCAFEBABE_DEADBEEF_u128;
        assert!(buf.insert(0x300, 8, val));
        let fwd = buf.forward(0x300, 8);
        assert_eq!(fwd, Some(0xCAFEBABE_DEADBEEF));
    }

    #[test]
    fn forward_no_overlap_returns_none() {
        let mut buf = StoreBuffer::new();
        assert!(buf.insert(0x100, 4, 0xAA));
        assert!(buf.forward(0x200, 4).is_none());
    }

    #[test]
    fn forward_partial_overlap_returns_none() {
        let mut buf = StoreBuffer::new();
        assert!(buf.insert(0x100, 2, 0xBBCC));
        assert!(buf.forward(0x100, 4).is_none());
    }

    #[test]
    fn most_recent_store_wins() {
        let mut buf = StoreBuffer::new();
        assert!(buf.insert(0x100, 4, 0x11111111));
        assert!(buf.insert(0x100, 4, 0x22222222));
        let fwd = buf.forward(0x100, 4);
        assert_eq!(fwd, Some(0x22222222));
    }

    #[test]
    fn wider_store_covers_narrower_load() {
        let mut buf = StoreBuffer::new();
        assert!(buf.insert(0x100, 8, 0x1122334455667788));
        let fwd = buf.forward(0x100, 4);
        assert_eq!(fwd, Some(0x11223344));
    }

    #[test]
    fn wider_store_covers_offset_load() {
        let mut buf = StoreBuffer::new();
        assert!(buf.insert(0x100, 8, 0x1122334455667788));
        let fwd = buf.forward(0x104, 4);
        assert_eq!(fwd, Some(0x55667788));
    }

    #[test]
    fn capacity_overflow_returns_false() {
        let mut buf = StoreBuffer::new();
        for i in 0..CAPACITY {
            assert!(buf.insert(i as u64 * 4, 4, i as u128));
        }
        assert!(buf.is_full());
        assert!(!buf.insert(0xFFFF, 4, 0));
    }

    #[test]
    fn clear_resets_buffer() {
        let mut buf = StoreBuffer::new();
        assert!(buf.insert(0x100, 4, 0xAA));
        assert_eq!(buf.len(), 1);
        buf.clear();
        assert!(buf.is_empty());
        assert!(buf.forward(0x100, 4).is_none());
    }

    #[test]
    fn flush_emits_effects_in_order() {
        let mut buf = StoreBuffer::new();
        assert!(buf.insert(0x100, 4, 0xAABBCCDD));
        assert!(buf.insert(0x200, 2, 0xEEFF));

        let mut effects = Vec::new();
        buf.flush(&mut effects, UnitId::new(0));
        assert_eq!(effects.len(), 2);
        assert!(buf.is_empty());

        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x100);
                assert_eq!(range.length(), 4);
                assert_eq!(bytes.bytes(), &[0xAA, 0xBB, 0xCC, 0xDD]);
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
        match &effects[1] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x200);
                assert_eq!(range.length(), 2);
                assert_eq!(bytes.bytes(), &[0xEE, 0xFF]);
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn has_store_in_range_detects_overlap() {
        let mut buf = StoreBuffer::new();
        assert!(buf.insert(0x100, 4, 0));
        assert!(buf.has_store_in_range(0x100, 0x200));
        assert!(buf.has_store_in_range(0x0, 0x104));
        assert!(!buf.has_store_in_range(0x200, 0x300));
        assert!(!buf.has_store_in_range(0x0, 0x100));
    }

    #[test]
    fn overlay_range_patches_only_overlapping_bytes() {
        // Region holds 0x11..0x20; one buffered 4-byte store at
        // 0x104 overrides bytes 4..8 of the 16-byte window.
        let mut buf = StoreBuffer::new();
        assert!(buf.insert(0x104, 4, 0xDEAD_BEEFu128));
        let mut out = [
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D,
            0x1E, 0x1F,
        ];
        buf.overlay_range(0x100, &mut out);
        assert_eq!(
            out,
            [
                0x10, 0x11, 0x12, 0x13, // unchanged
                0xDE, 0xAD, 0xBE, 0xEF, // patched
                0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F,
            ]
        );
    }

    #[test]
    fn overlay_range_later_store_wins_in_overlap() {
        let mut buf = StoreBuffer::new();
        assert!(buf.insert(0x100, 4, 0x1111_1111u128));
        assert!(buf.insert(0x102, 2, 0x2222u128)); // overlaps the upper half of the first
        let mut out = [0u8; 8];
        buf.overlay_range(0x100, &mut out);
        // First store: 0x11 0x11 0x11 0x11 at offset 0..4.
        // Second store: 0x22 0x22 at offset 2..4 -- overwrites.
        assert_eq!(out[0..4], [0x11, 0x11, 0x22, 0x22]);
        assert_eq!(out[4..8], [0, 0, 0, 0]);
    }

    #[test]
    fn overlay_range_skips_entries_outside_window() {
        let mut buf = StoreBuffer::new();
        assert!(buf.insert(0x80, 4, 0xDEAD_BEEFu128)); // before window
        assert!(buf.insert(0x200, 4, 0xCAFE_BABEu128)); // after window
        let mut out = [0xAAu8; 16];
        buf.overlay_range(0x100, &mut out);
        assert_eq!(out, [0xAA; 16]);
    }

    #[test]
    fn forward_u16_from_u32_store() {
        let mut buf = StoreBuffer::new();
        assert!(buf.insert(0x100, 4, 0xAABBCCDD));
        let fwd = buf.forward(0x100, 2);
        assert_eq!(fwd, Some(0xAABB));
        let fwd = buf.forward(0x102, 2);
        assert_eq!(fwd, Some(0xCCDD));
    }

    #[test]
    fn forward_single_byte_from_u32_store() {
        let mut buf = StoreBuffer::new();
        assert!(buf.insert(0x100, 4, 0xAABBCCDD));
        assert_eq!(buf.forward(0x100, 1), Some(0xAA));
        assert_eq!(buf.forward(0x101, 1), Some(0xBB));
        assert_eq!(buf.forward(0x102, 1), Some(0xCC));
        assert_eq!(buf.forward(0x103, 1), Some(0xDD));
    }

    #[test]
    fn insert_u16_vector_store() {
        let mut buf = StoreBuffer::new();
        let val = 0x0102030405060708090A0B0C0D0E0F10_u128;
        assert!(buf.insert(0x100, 16, val));
        let fwd = buf.forward(0x100, 16);
        assert_eq!(fwd, Some(val));
    }

    #[test]
    fn flush_skips_conditional_entries() {
        // A conditional entry must never produce a `SharedWriteIntent`
        // -- `stwcx`/`stdcx` already emitted its own `ConditionalStore`
        // effect, and double-emission would re-run the reservation
        // clear-sweep against the same write.
        let mut buf = StoreBuffer::new();
        assert!(buf.insert(0x100, 4, 0xAABBCCDD));
        assert!(buf.insert_conditional(0x200, 4, 0x11223344));
        assert!(buf.insert(0x300, 2, 0xEEFF));

        let mut effects = Vec::new();
        buf.flush(&mut effects, UnitId::new(0));
        assert!(buf.is_empty());
        assert_eq!(effects.len(), 2);
        match &effects[0] {
            Effect::SharedWriteIntent { range, .. } => {
                assert_eq!(range.start().raw(), 0x100);
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
        match &effects[1] {
            Effect::SharedWriteIntent { range, .. } => {
                assert_eq!(range.start().raw(), 0x300);
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn overlay_range_applies_conditional_entry() {
        // Within a single step the context is frozen, so committed
        // memory has not yet absorbed the `ConditionalStore`. The
        // buffered conditional entry is the sole intra-step record
        // of those bytes -- a vector load overlapping the range must
        // see them.
        let mut buf = StoreBuffer::new();
        assert!(buf.insert_conditional(0x104, 4, 0xCAFE_BABE_u128));
        let mut out = [0xAAu8; 16];
        buf.overlay_range(0x100, &mut out);
        assert_eq!(out[0..4], [0xAA, 0xAA, 0xAA, 0xAA]);
        assert_eq!(out[4..8], [0xCA, 0xFE, 0xBA, 0xBE]);
        assert_eq!(out[8..16], [0xAA; 8]);
    }

    #[test]
    fn overlay_range_entry_starts_before_window_extends_in() {
        // Store at 0xFE..0x102 -- straddles the low edge of a window
        // anchored at 0x100. Only the bytes >= base must be applied.
        let mut buf = StoreBuffer::new();
        assert!(buf.insert(0xFE, 4, 0x1122_3344_u128));
        let mut out = [0xAAu8; 8];
        buf.overlay_range(0x100, &mut out);
        // Entry value bytes (BE): 11 22 33 44 covering EAs FE FF 100 101.
        // The window starts at 100, so only bytes 0x33 (EA 100) and
        // 0x44 (EA 101) fall inside.
        assert_eq!(out[0..2], [0x33, 0x44]);
        assert_eq!(out[2..8], [0xAA; 6]);
    }

    #[test]
    fn overlay_range_entry_starts_in_window_extends_past_end() {
        // Store at 0x106..0x10A -- straddles the high edge of the
        // 8-byte window at 0x100..0x108. Only bytes < base_end apply.
        let mut buf = StoreBuffer::new();
        assert!(buf.insert(0x106, 4, 0xDEAD_BEEF_u128));
        let mut out = [0xAAu8; 8];
        buf.overlay_range(0x100, &mut out);
        // Entry covers EAs 106 107 108 109 with bytes DE AD BE EF.
        // Only EAs 106 and 107 are inside the window.
        assert_eq!(out[0..6], [0xAA; 6]);
        assert_eq!(out[6..8], [0xDE, 0xAD]);
    }
}
