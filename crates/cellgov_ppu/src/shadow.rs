//! Predecoded instruction shadow for PT_LOAD text ranges.
//!
//! Decodes every instruction word in a guest memory range once at
//! construction time, storing the result in a flat
//! `Vec<PpuInstruction>` indexed by `(pc - base) / 4`. The PPU's
//! hot-path fetch becomes a bounds check + array index instead of
//! a raw-memory read + `decode::decode` match cascade.
//!
//! Invalidation: callers mark a byte range stale after any commit
//! that writes into the shadowed region. The next fetch of a stale
//! slot re-decodes from committed memory and clears the stale bit.
//! Self-modifying code (CRT0 relocations, HLE trampoline planting)
//! works through this path.

use crate::decode;
use crate::instruction::PpuInstruction;

/// A predecoded instruction shadow covering one contiguous guest
/// memory range. Each 4-byte-aligned slot holds either a valid
/// `PpuInstruction` or a stale marker that forces re-decode on
/// the next fetch.
pub struct PredecodedShadow {
    base: u64,
    /// Decoded instructions. `Some(insn)` for successfully decoded
    /// words, `None` for words that failed to decode. A `None`
    /// slot behaves the same as a stale slot: the caller falls
    /// back to raw fetch + decode, which will produce the same
    /// decode error the non-shadowed path would.
    slots: Vec<Option<PpuInstruction>>,
    stale: Vec<bool>,
}

impl PredecodedShadow {
    /// Build a shadow from raw guest memory bytes.
    ///
    /// `base` is the guest address of `bytes[0]`. Every aligned
    /// 4-byte word in `bytes` is decoded; words that fail to
    /// decode store `None` (the caller falls back to the raw
    /// fetch + decode path, producing the same decode-error fault
    /// the non-shadowed interpreter would).
    pub fn build(base: u64, bytes: &[u8]) -> Self {
        let n_slots = bytes.len() / 4;
        let mut slots = Vec::with_capacity(n_slots);
        for i in 0..n_slots {
            let off = i * 4;
            let raw =
                u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
            slots.push(decode::decode(raw).ok());
        }
        let stale = vec![false; n_slots];
        Self { base, slots, stale }
    }

    /// Guest base address of the shadowed range.
    #[inline]
    pub fn base(&self) -> u64 {
        self.base
    }

    /// Number of instruction slots.
    #[inline]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether the shadow has no slots.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// End address (exclusive) of the shadowed range.
    #[inline]
    pub fn end(&self) -> u64 {
        self.base + (self.slots.len() as u64) * 4
    }

    /// Fetch the instruction at `pc`. Returns `Some(insn)` if the
    /// slot is valid (not stale, successfully decoded) and `pc`
    /// falls inside the shadowed range on a 4-byte boundary.
    /// Returns `None` if the slot is stale, was a decode error,
    /// out of range, or misaligned. A `None` return tells the
    /// caller to fall back to the raw fetch + decode path.
    #[inline]
    pub fn get(&self, pc: u64) -> Option<PpuInstruction> {
        if pc < self.base {
            return None;
        }
        let byte_offset = pc - self.base;
        if byte_offset & 3 != 0 {
            return None;
        }
        let idx = (byte_offset / 4) as usize;
        if idx >= self.slots.len() {
            return None;
        }
        if self.stale[idx] {
            return None;
        }
        self.slots[idx]
    }

    /// Mark every slot overlapping the byte range
    /// `[addr, addr + len)` as stale. The next `get` for those
    /// PCs will return `None`, forcing the caller to re-decode
    /// from committed memory via [`refresh`](Self::refresh).
    pub fn invalidate_range(&mut self, addr: u64, len: u64) {
        if len == 0 {
            return;
        }
        let end = addr.saturating_add(len);
        let shadow_end = self.end();
        if addr >= shadow_end || end <= self.base {
            return;
        }
        let clamp_lo = addr.max(self.base);
        let clamp_hi = end.min(shadow_end);
        let first_slot = ((clamp_lo - self.base) / 4) as usize;
        let last_slot = ((clamp_hi - self.base) as usize).div_ceil(4);
        let last_slot = last_slot.min(self.slots.len());
        for i in first_slot..last_slot {
            self.stale[i] = true;
        }
    }

    /// Re-decode a single slot from a raw instruction word and
    /// clear its stale bit. `pc` must be inside the shadow and
    /// 4-byte aligned. Returns `None` if the slot is out of range
    /// or misaligned; `Some(None)` if the decode failed (the slot
    /// stores `None`, matching the build-time decode-error path);
    /// `Some(Some(insn))` on a successful re-decode.
    pub fn refresh(&mut self, pc: u64, raw: u32) -> Option<Option<PpuInstruction>> {
        if pc < self.base {
            return None;
        }
        let byte_offset = pc - self.base;
        if byte_offset & 3 != 0 {
            return None;
        }
        let idx = (byte_offset / 4) as usize;
        if idx >= self.slots.len() {
            return None;
        }
        let insn = decode::decode(raw).ok();
        self.slots[idx] = insn;
        self.stale[idx] = false;
        Some(insn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn li_raw(rd: u32, simm: i16) -> u32 {
        (14 << 26) | (rd << 21) | ((simm as u16) as u32)
    }

    fn build_from_words(base: u64, words: &[u32]) -> PredecodedShadow {
        let mut bytes = Vec::with_capacity(words.len() * 4);
        for &w in words {
            bytes.extend_from_slice(&w.to_be_bytes());
        }
        PredecodedShadow::build(base, &bytes)
    }

    #[test]
    fn build_decodes_all_slots() {
        let shadow = build_from_words(0, &[li_raw(3, 10), li_raw(4, 20)]);
        assert_eq!(shadow.len(), 2);
        assert_eq!(shadow.base(), 0);
        assert_eq!(shadow.end(), 8);
        assert!(shadow.get(0).is_some());
        assert!(shadow.get(4).is_some());
    }

    #[test]
    fn get_returns_none_for_out_of_range() {
        let shadow = build_from_words(0x100, &[li_raw(3, 42)]);
        assert!(shadow.get(0x0FC).is_none());
        assert!(shadow.get(0x100).is_some());
        assert!(shadow.get(0x104).is_none());
    }

    #[test]
    fn get_returns_none_for_misaligned_pc() {
        let shadow = build_from_words(0, &[li_raw(3, 1), li_raw(4, 2)]);
        assert!(shadow.get(2).is_none());
    }

    #[test]
    fn invalidate_marks_slots_stale() {
        let mut shadow = build_from_words(0, &[li_raw(3, 10), li_raw(4, 20), li_raw(5, 30)]);
        assert!(shadow.get(0).is_some());
        assert!(shadow.get(4).is_some());
        assert!(shadow.get(8).is_some());

        shadow.invalidate_range(2, 4);
        // Byte range 2..6 overlaps slots at offset 0 and 4.
        assert!(shadow.get(0).is_none(), "slot 0 should be stale");
        assert!(shadow.get(4).is_none(), "slot 1 should be stale");
        assert!(shadow.get(8).is_some(), "slot 2 untouched");
    }

    #[test]
    fn invalidate_outside_range_is_noop() {
        let mut shadow = build_from_words(0x100, &[li_raw(3, 1)]);
        shadow.invalidate_range(0, 0x100);
        assert!(shadow.get(0x100).is_some());
        shadow.invalidate_range(0x200, 0x100);
        assert!(shadow.get(0x100).is_some());
    }

    #[test]
    fn refresh_clears_stale_and_updates_slot() {
        let mut shadow = build_from_words(0, &[li_raw(3, 10)]);
        shadow.invalidate_range(0, 4);
        assert!(shadow.get(0).is_none());

        let new_raw = li_raw(3, 99);
        let insn = shadow.refresh(0, new_raw);
        assert_eq!(insn, Some(Some(decode::decode(new_raw).unwrap())));
        assert!(shadow.get(0).is_some());
    }

    #[test]
    fn refresh_out_of_range_returns_none() {
        let mut shadow = build_from_words(0x100, &[li_raw(3, 1)]);
        assert!(shadow.refresh(0, li_raw(3, 1)).is_none());
        assert!(shadow.refresh(0x104, li_raw(3, 1)).is_none());
    }

    #[test]
    fn empty_shadow_is_empty() {
        let shadow = PredecodedShadow::build(0, &[]);
        assert!(shadow.is_empty());
        assert_eq!(shadow.len(), 0);
        assert!(shadow.get(0).is_none());
    }

    #[test]
    fn invalidate_zero_length_is_noop() {
        let mut shadow = build_from_words(0, &[li_raw(3, 1)]);
        shadow.invalidate_range(0, 0);
        assert!(shadow.get(0).is_some());
    }

    #[test]
    fn invalidate_partial_byte_within_slot_stales_that_slot() {
        let mut shadow = build_from_words(0, &[li_raw(3, 1)]);
        shadow.invalidate_range(3, 1);
        assert!(
            shadow.get(0).is_none(),
            "1-byte write inside slot must stale it"
        );
    }
}
