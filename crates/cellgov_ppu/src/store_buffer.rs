//! Intra-block store-forwarding buffer.
//!
//! Tracks pending stores within a basic-block execution window so
//! that subsequent loads can see values written earlier in the same
//! block before they are committed to guest memory. The buffer is
//! flushed to `Effect::SharedWriteIntent` packets at block
//! boundaries and cleared between steps.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_time::GuestTicks;

const CAPACITY: usize = 64;

/// A single pending store entry.
///
/// `conditional` is true for entries that carry the bytes of a
/// successful `stwcx` / `stdcx`. The flush pass skips these (the
/// ConditionalStore effect was emitted separately), but the
/// forwarding pass sees them so a subsequent same-step lwarx
/// observes its own prior conditional-store's bytes.
#[derive(Clone, Copy)]
struct StoreEntry {
    addr: u64,
    len: u8,
    conditional: bool,
    value: u128,
}

/// Fixed-capacity store-forwarding buffer.
///
/// Stores are appended in program order. Loads check the buffer
/// via reverse scan (most-recent store wins). At block end the
/// buffer is flushed to effects and cleared.
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

    /// Clear all entries.
    #[inline]
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Insert a pending store. Returns false if the buffer is full
    /// (the caller should yield the block before retrying).
    #[inline]
    pub fn insert(&mut self, addr: u64, len: u8, value: u128) -> bool {
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

    /// Insert a successful conditional store (stwcx / stdcx) for
    /// intra-step forwarding only. The flush pass skips these
    /// entries because the ConditionalStore effect was emitted
    /// directly by the instruction handler; this insert just lets
    /// a subsequent same-step lwarx see its own prior bytes.
    /// Returns false if the buffer is full.
    #[inline]
    pub fn insert_conditional(&mut self, addr: u64, len: u8, value: u128) -> bool {
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

    /// Try to forward a load from the buffer. Scans in reverse
    /// (most-recent store wins). Returns `Some(value)` if a pending
    /// store fully covers the load range `[addr, addr+len)`.
    /// Returns `None` if no covering store exists (caller falls
    /// back to committed memory).
    ///
    /// Partial overlaps are not forwarded -- the caller reads from
    /// committed memory, which is correct because committed memory
    /// contains the pre-block state and no partial store has been
    /// committed yet. A future update can add byte-merge for
    /// partial overlaps if profiling shows it matters.
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

    /// Flush all pending stores to the effects vec as
    /// `SharedWriteIntent` packets in program order, then clear.
    pub fn flush(&mut self, effects: &mut Vec<Effect>, source: UnitId) {
        for i in 0..self.entries.len() {
            let e = &self.entries[i];
            if e.conditional {
                // ConditionalStore effects are emitted separately
                // by stwcx / stdcx. The buffer entry exists only
                // for intra-step forwarding; do not double-emit.
                continue;
            }
            let bytes = &e.value.to_be_bytes();
            let offset = 16 - e.len as usize;
            let payload = WritePayload::from_slice(&bytes[offset..]);
            if let Some(range) = ByteRange::new(GuestAddr::new(e.addr), e.len as u64) {
                effects.push(Effect::SharedWriteIntent {
                    range,
                    bytes: payload,
                    ordering: PriorityClass::Normal,
                    source,
                    source_time: GuestTicks::ZERO,
                });
            }
        }
        self.entries.clear();
    }

    /// Check whether any pending store targets an address within
    /// the given shadow range `[shadow_base, shadow_end)`. Used to
    /// detect stores to code regions that require an early block yield.
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
        // Store 0xDEADBEEF at addr 0x100, 4 bytes
        let val = 0xDEADBEEF_u128;
        assert!(buf.insert(0x100, 4, val));
        assert_eq!(buf.len(), 1);

        // Forward a 4-byte load at the same address
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
        // Store 2 bytes at 0x100
        assert!(buf.insert(0x100, 2, 0xBBCC));
        // Load 4 bytes at 0x100 -- partial overlap, not forwarded
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
        // Store 8 bytes at 0x100
        assert!(buf.insert(0x100, 8, 0x1122334455667788));
        // Load 4 bytes at 0x100 -- covered by the 8-byte store
        let fwd = buf.forward(0x100, 4);
        assert_eq!(fwd, Some(0x11223344));
    }

    #[test]
    fn wider_store_covers_offset_load() {
        let mut buf = StoreBuffer::new();
        // Store 8 bytes at 0x100
        assert!(buf.insert(0x100, 8, 0x1122334455667788));
        // Load 4 bytes at 0x104 -- middle of the 8-byte store
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
    fn forward_u16_from_u32_store() {
        let mut buf = StoreBuffer::new();
        // Store 4 bytes: value 0xAABBCCDD at addr 0x100
        assert!(buf.insert(0x100, 4, 0xAABBCCDD));
        // Load 2 bytes at 0x100 -- should get high halfword
        let fwd = buf.forward(0x100, 2);
        assert_eq!(fwd, Some(0xAABB));
        // Load 2 bytes at 0x102 -- should get low halfword
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
}
