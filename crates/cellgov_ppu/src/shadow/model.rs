//! [`PredecodedShadow`] state and per-method impls.
//!
//! Quickening + super-pair passes live in [`super::quicken`] and
//! [`super::superpair`] respectively.

use crate::decode;
use crate::instruction::PpuInstruction;
use crate::shadow::quicken::quicken_insn;
use crate::shadow::superpair::make_super_pair;

/// Predecoded instruction shadow covering one contiguous guest range.
#[derive(Clone)]
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

impl PredecodedShadow {
    /// Build a shadow from raw guest memory bytes.
    ///
    /// `base` is the guest address of `bytes[0]`. Decode is followed
    /// by a quickening pass and a super-pairing pass; block lengths
    /// are recomputed once super-pairing has fixed terminator status
    /// for the fused variants. Any PC outside `[base, base +
    /// bytes.len())` falls back to live decode + `refresh` on the
    /// hot path.
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
                Some(insn) if !insn.is_block_terminator() => {
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
    /// Quickening is re-applied; super-pairing is not. Two slots
    /// whose refreshed content forms a fusable pair will run as
    /// separate dispatches until the next full shadow rebuild.
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
            Some(insn) => insn.is_block_terminator(),
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
                Some(insn) if !insn.is_block_terminator() => {
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
#[path = "tests/model_tests.rs"]
mod tests;
