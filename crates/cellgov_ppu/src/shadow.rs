//! Predecoded instruction shadow for PT_LOAD text ranges.
//!
//! Fetch is a bounds check plus array index; slots are produced once
//! at construction, with a quickening and super-pairing pass applied
//! before first use.
//!
//! Self-modifying code (CRT0 relocations, HLE trampoline planting)
//! goes through [`invalidate_range`](PredecodedShadow::invalidate_range)
//! followed by [`refresh`](PredecodedShadow::refresh); the stale bit
//! forces the caller onto the raw fetch + decode path until the
//! slot is repopulated from committed memory.
//!
//! The two passes that run during `build` live in their own
//! submodules so each can be tested in isolation:
//!
//! - [`quicken`]: rewrite a single decoded instruction into a
//!   specialized variant (e.g. `addi r3, 0, imm` -> `Li`).
//! - [`superpair`]: fuse two adjacent instructions into a single
//!   super-instruction variant (e.g. `lwz` + `cmpwi` -> `LwzCmpwi`),
//!   replacing the second slot with `Consumed`.

mod quicken;
mod superpair;
#[cfg(test)]
mod test_support;

use crate::decode;
use crate::instruction::PpuInstruction;

use quicken::quicken_insn;
use superpair::make_super_pair;

/// Predecoded instruction shadow covering one contiguous guest range.
pub struct PredecodedShadow {
    base: u64,
    /// `None` means decode failed; callers treat it like a stale slot
    /// and fall through to the raw fetch + decode path.
    slots: Vec<Option<PpuInstruction>>,
    stale: Vec<bool>,
    /// Instructions remaining to end of basic block (inclusive).
    /// Stale slots collapse to 1 (conservative single-instruction block).
    block_len: Vec<u16>,
}

fn is_block_terminator(insn: &PpuInstruction) -> bool {
    matches!(
        insn,
        PpuInstruction::B { .. }
            | PpuInstruction::Bc { .. }
            | PpuInstruction::Bclr { .. }
            | PpuInstruction::Bcctr { .. }
            | PpuInstruction::CmpwiBc { .. }
            | PpuInstruction::CmpwBc { .. }
            | PpuInstruction::Sc { .. }
    )
}

impl PredecodedShadow {
    /// Build a shadow from raw guest memory bytes.
    ///
    /// `base` is the guest address of `bytes[0]`. Decode is followed
    /// by a quickening pass and a super-pairing pass; block lengths
    /// are recomputed once super-pairing has fixed terminator status
    /// for the fused variants.
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
        let block_len = Self::compute_block_lengths(&slots);
        let mut shadow = Self {
            base,
            slots,
            stale,
            block_len,
        };
        shadow.quicken();
        shadow.super_pair();
        // CmpwiBc/CmpwBc promote non-terminators to terminators.
        shadow.block_len = Self::compute_block_lengths(&shadow.slots);
        shadow
    }

    fn compute_block_lengths(slots: &[Option<PpuInstruction>]) -> Vec<u16> {
        let n = slots.len();
        let mut bl = vec![1u16; n];
        if n == 0 {
            return bl;
        }
        // End of shadow is an implicit block boundary.
        for i in (0..n.saturating_sub(1)).rev() {
            match &slots[i] {
                Some(insn) if !is_block_terminator(insn) => {
                    bl[i] = bl[i + 1].saturating_add(1);
                }
                _ => {
                    bl[i] = 1;
                }
            }
        }
        bl
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

    /// Fetch the instruction at `pc`.
    ///
    /// `None` means the caller must fall back to raw fetch + decode:
    /// slot is stale, decode failed at build time, `pc` is out of
    /// range, or `pc` is misaligned.
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

    /// Mark every slot overlapping `[addr, addr + len)` as stale.
    ///
    /// Super-pair partners are widened in: invalidating just the
    /// `Consumed` half of a fused pair would leave the head's fused
    /// operation still firing for an instruction that may have been
    /// rewritten; invalidating just the head would leave an orphan
    /// `Consumed` that the fetch loop skips without executing what
    /// was actually written there. Both halves are staled as a unit
    /// so the runtime falls back to raw fetch + decode for either
    /// side until the caller refreshes them.
    ///
    /// Predecessor `block_len` entries are not recomputed here; they
    /// may overshoot, but the per-slot `stale`/`None` check on each
    /// fetch re-bounds the block. Pair with [`refresh`](Self::refresh)
    /// to repopulate once new bytes are committed.
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
        let mut first_slot = ((clamp_lo - self.base) / 4) as usize;
        let mut last_slot = ((clamp_hi - self.base) as usize).div_ceil(4);
        last_slot = last_slot.min(self.slots.len());

        // Super-pair partner widening. A super-pair has its head at
        // index k and a `Consumed` placeholder at k+1; the two slots
        // must transition to stale together or the fused dispatch and
        // the freshly-written byte at the partner slot disagree.
        if first_slot > 0 && matches!(self.slots[first_slot], Some(PpuInstruction::Consumed)) {
            first_slot -= 1;
        }
        if last_slot < self.slots.len()
            && matches!(self.slots[last_slot], Some(PpuInstruction::Consumed))
        {
            last_slot += 1;
        }

        for i in first_slot..last_slot {
            self.stale[i] = true;
            self.block_len[i] = 1;
        }
    }

    /// Re-decode a single slot and clear its stale bit.
    ///
    /// Outer `None` means `pc` is out of range or misaligned.
    /// `Some(None)` means decode failed (same slot state as the
    /// build-time decode-error path).
    ///
    /// Quickening is re-applied; super-pairing is intentionally not.
    /// Re-pairing on a single refreshed slot would have to inspect
    /// both neighbors, possibly tear down or re-form an adjacent
    /// fusion, and reason about a partner that may itself be stale
    /// or freshly written. The cost is that two slots whose new
    /// content forms a fusable pair (e.g., post-relocation `lwz +
    /// cmpwi`) will run as separate dispatches until a full shadow
    /// rebuild. The correctness vs perf tradeoff favors correctness:
    /// a partly-rewritten fusion state is harder to reason about
    /// than a missed optimization.
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
        let quickened = insn.map(|i| quicken_insn(i).unwrap_or(i));
        self.slots[idx] = quickened;
        self.stale[idx] = false;
        self.rescan_block_len(idx);
        Some(quickened)
    }

    /// Instructions remaining in the basic block at `pc` (inclusive).
    ///
    /// Upper bound only: invalidation without a subsequent refresh
    /// leaves predecessor counts unchanged. Returns 1 for out-of-range
    /// or misaligned `pc`.
    #[inline]
    pub fn block_len_at(&self, pc: u64) -> u16 {
        if pc < self.base {
            return 1;
        }
        let byte_offset = pc - self.base;
        if byte_offset & 3 != 0 {
            return 1;
        }
        let idx = (byte_offset / 4) as usize;
        if idx >= self.block_len.len() {
            return 1;
        }
        self.block_len[idx]
    }

    fn rescan_block_len(&mut self, idx: usize) {
        let is_term = match &self.slots[idx] {
            Some(insn) => is_block_terminator(insn),
            None => true,
        };
        if is_term || idx + 1 >= self.slots.len() || self.stale.get(idx + 1) == Some(&true) {
            self.block_len[idx] = 1;
        } else {
            self.block_len[idx] = self.block_len[idx + 1].saturating_add(1);
        }
        let mut i = idx;
        while i > 0 {
            i -= 1;
            if self.stale[i] {
                break;
            }
            match &self.slots[i] {
                Some(insn) if !is_block_terminator(insn) => {
                    self.block_len[i] = self.block_len[i + 1].saturating_add(1);
                }
                _ => break,
            }
        }
    }

    /// Rewrite decoded slots in place into specialized instruction
    /// variants. Run once during [`build`](Self::build); not exposed
    /// externally since the shadow's invariants assume quickening
    /// has already happened by the time `get` is called.
    fn quicken(&mut self) {
        for i in 0..self.slots.len() {
            if self.stale[i] {
                continue;
            }
            if let Some(insn) = self.slots[i] {
                if let Some(quick) = quicken_insn(insn) {
                    self.slots[i] = Some(quick);
                }
            }
        }
    }

    /// Fuse adjacent instruction pairs into superinstructions.
    ///
    /// Runs after [`quicken`](Self::quicken); the second slot of a
    /// fused pair is replaced with `Consumed`. Pairs that cross a
    /// block boundary (first operand is a terminator) are skipped,
    /// but fusions that *produce* a terminator (CmpwiBc/CmpwBc) are
    /// allowed -- the caller in [`build`](Self::build) recomputes
    /// `block_len` after this runs.
    fn super_pair(&mut self) {
        let n = self.slots.len();
        if n < 2 {
            return;
        }
        let mut i = 0;
        while i < n - 1 {
            if self.stale[i] || self.stale[i + 1] {
                i += 1;
                continue;
            }
            let (a, b) = match (self.slots[i], self.slots[i + 1]) {
                (Some(a), Some(b)) => (a, b),
                _ => {
                    i += 1;
                    continue;
                }
            };
            if let Some(super_insn) = make_super_pair(a, b) {
                self.slots[i] = Some(super_insn);
                self.slots[i + 1] = Some(PpuInstruction::Consumed);
                // Skip Consumed so it is not fused with slot i+2.
                i += 2;
            } else {
                i += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{
        b_raw, build_from_words, cmpwi_raw, li_raw, lwz_raw, sc_raw, stw_raw,
    };
    use super::*;

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

        // Byte range 2..6 overlaps slots 0 and 1.
        shadow.invalidate_range(2, 4);
        assert!(shadow.get(0).is_none());
        assert!(shadow.get(4).is_none());
        assert!(shadow.get(8).is_some());
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
        // Quickening applies: addi r3, r0, 99 => Li.
        assert_eq!(insn, Some(Some(PpuInstruction::Li { rt: 3, imm: 99 })));
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

    #[test]
    fn block_len_straight_line() {
        let shadow = build_from_words(0, &[li_raw(3, 1), li_raw(4, 2), li_raw(5, 3), li_raw(6, 4)]);
        assert_eq!(shadow.block_len_at(0), 4);
        assert_eq!(shadow.block_len_at(4), 3);
        assert_eq!(shadow.block_len_at(8), 2);
        assert_eq!(shadow.block_len_at(12), 1);
    }

    #[test]
    fn block_len_branch_terminates() {
        let shadow = build_from_words(0, &[li_raw(3, 1), li_raw(4, 2), b_raw(8), li_raw(5, 3)]);
        assert_eq!(shadow.block_len_at(0), 3);
        assert_eq!(shadow.block_len_at(4), 2);
        assert_eq!(shadow.block_len_at(8), 1);
        assert_eq!(shadow.block_len_at(12), 1);
    }

    #[test]
    fn block_len_syscall_terminates() {
        let shadow = build_from_words(0, &[li_raw(3, 1), sc_raw(), li_raw(4, 2)]);
        assert_eq!(shadow.block_len_at(0), 2);
        assert_eq!(shadow.block_len_at(4), 1);
        assert_eq!(shadow.block_len_at(8), 1);
    }

    #[test]
    fn block_len_invalidation_resets() {
        let mut shadow = build_from_words(0, &[li_raw(3, 1), li_raw(4, 2), li_raw(5, 3)]);
        assert_eq!(shadow.block_len_at(0), 3);
        shadow.invalidate_range(4, 4);
        // Predecessor block_len is an upper bound post-invalidation.
        assert_eq!(shadow.block_len_at(0), 3);
        assert_eq!(shadow.block_len_at(4), 1);
        assert_eq!(shadow.block_len_at(8), 1);
    }

    #[test]
    fn block_len_refresh_rescans() {
        let mut shadow = build_from_words(0, &[li_raw(3, 1), li_raw(4, 2), li_raw(5, 3)]);
        assert_eq!(shadow.block_len_at(0), 3);
        shadow.invalidate_range(4, 4);
        assert_eq!(shadow.block_len_at(4), 1);
        shadow.refresh(4, li_raw(4, 99));
        assert_eq!(shadow.block_len_at(4), 2);
        assert_eq!(shadow.block_len_at(0), 3);
    }

    #[test]
    fn block_len_out_of_range_returns_one() {
        let shadow = build_from_words(0x100, &[li_raw(3, 1)]);
        assert_eq!(shadow.block_len_at(0), 1);
        assert_eq!(shadow.block_len_at(0x200), 1);
    }

    #[test]
    fn block_len_empty_shadow() {
        let shadow = PredecodedShadow::build(0, &[]);
        assert_eq!(shadow.block_len_at(0), 1);
    }

    #[test]
    fn invalidate_just_consumed_widens_to_super_pair_head() {
        // lwz + cmpwi fused at slot 0, Consumed at slot 4. Invalidating
        // only the Consumed slot must widen to include the head;
        // otherwise the head's fused dispatch would still execute (firing
        // both the lwz and the cmpwi) and the freshly-written instruction
        // at slot 4 would also execute -- a double-execute.
        let mut shadow = build_from_words(0, &[lwz_raw(3, 1, 8), cmpwi_raw(0, 3, 42)]);
        assert!(matches!(
            shadow.get(0),
            Some(PpuInstruction::LwzCmpwi { .. })
        ));
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
        // Invalidate ONLY slot 4 (byte range [4..8)).
        shadow.invalidate_range(4, 4);
        assert!(
            shadow.get(0).is_none(),
            "super-pair head must be staled when its Consumed partner is invalidated"
        );
        assert!(shadow.get(4).is_none());
    }

    #[test]
    fn invalidate_just_super_pair_head_widens_to_consumed() {
        // Symmetric case: invalidate only the head (slot 0) and verify
        // the Consumed at slot 4 also goes stale. Otherwise the fetch
        // loop would skip slot 4 forever (treating Consumed as the
        // already-retired second half of a pair the head no longer
        // represents).
        let mut shadow = build_from_words(0, &[lwz_raw(3, 1, 8), cmpwi_raw(0, 3, 42)]);
        assert!(matches!(
            shadow.get(0),
            Some(PpuInstruction::LwzCmpwi { .. })
        ));
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
        // Invalidate ONLY slot 0 (byte range [0..4)).
        shadow.invalidate_range(0, 4);
        assert!(shadow.get(0).is_none());
        assert!(
            shadow.get(4).is_none(),
            "Consumed partner must be staled when its super-pair head is invalidated"
        );
    }

    #[test]
    fn invalidate_partner_widening_preserves_unrelated_pairs() {
        // Two adjacent pairs at slots 0..2 and 2..4. Invalidating only
        // slot 1 (the Consumed of pair A) must widen to slot 0 (head of
        // pair A) but not touch pair B at slots 2..4.
        let mut shadow = build_from_words(
            0,
            &[
                li_raw(3, 1),
                stw_raw(3, 1, 0),
                li_raw(4, 2),
                stw_raw(4, 1, 8),
            ],
        );
        assert!(matches!(shadow.get(0), Some(PpuInstruction::LiStw { .. })));
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
        assert!(matches!(shadow.get(8), Some(PpuInstruction::LiStw { .. })));
        assert_eq!(shadow.get(12), Some(PpuInstruction::Consumed));
        // Invalidate only slot 1 (the Consumed at byte 4).
        shadow.invalidate_range(4, 4);
        assert!(shadow.get(0).is_none(), "pair A head must be staled");
        assert!(shadow.get(4).is_none(), "pair A Consumed must be staled");
        assert!(shadow.get(8).is_some(), "pair B head must remain intact");
        assert_eq!(
            shadow.get(12),
            Some(PpuInstruction::Consumed),
            "pair B Consumed must remain intact"
        );
    }
}
