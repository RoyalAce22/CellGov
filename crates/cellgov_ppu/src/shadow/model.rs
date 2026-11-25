//! [`PredecodedShadow`] state and per-method impls.
//!
//! Quickening + super-pair passes live in [`super::quicken`] and
//! [`super::superpair`] respectively.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::decode;
use crate::instruction::PpuInstruction;
use crate::shadow::quicken::quicken_insn;
use crate::shadow::superpair::make_super_pair;

/// Predecoded instruction shadow covering one contiguous guest range.
///
/// A clone shares the build-time decode and the written flag of each slot
/// with its source. A clone therefore costs a reference count and a copy
/// of the slots that the source refreshed. The units of one address space
/// and each runtime snapshot hold such clones.
///
/// A slot of one shadow reads as:
/// - its build-time decode, while its written flag is clear;
/// - else the decode this shadow refreshed it with, until this shadow
///   marks the slot stale again;
/// - else stale.
///
/// No operation clears a written flag. When one shadow gets an
/// invalidation or a refresh over a slot, the slot goes stale in each
/// sharing shadow that did not refresh it. A fetch through a stale slot
/// decodes the stored word, so it never runs an older decode.
#[derive(Clone)]
pub struct PredecodedShadow {
    shared: Arc<Decoded>,
    /// The slots this shadow refreshed, each with its decode and its stale
    /// flag. The shared written flag of each key is set.
    refreshed: BTreeMap<usize, (Option<PpuInstruction>, bool)>,
}

/// The part of a shadow that each clone shares.
struct Decoded {
    base: u64,
    /// `None` means decode failed; callers treat it like a stale slot
    /// and fall through to the raw fetch + decode path.
    slots: Box<[Option<PpuInstruction>]>,
    /// Whether a sharing shadow changed the slot, by one of:
    /// - an invalidation over the slot;
    /// - a refresh of the slot;
    /// - a refresh of the other half of its fused pair.
    ///
    /// No operation clears a flag.
    ///
    /// The runtime steps its units on one host thread, so a flag orders
    /// no other memory and every access is `Relaxed`.
    written: Box<[AtomicBool]>,
}

impl PredecodedShadow {
    /// Build a shadow from raw guest memory bytes.
    ///
    /// `base` is the guest address of `bytes[0]`. Decode is followed
    /// by a quickening pass and a super-pairing pass. Any PC outside
    /// `[base, base + bytes.len())` falls back to live decode +
    /// `refresh` on the hot path.
    pub fn build(base: u64, bytes: &[u8]) -> Self {
        Self::build_with_fusion(base, bytes, true)
    }

    /// Builds a shadow with quickening but without fused pairs.
    pub fn build_quickened(base: u64, bytes: &[u8]) -> Self {
        Self::build_with_fusion(base, bytes, false)
    }

    fn build_with_fusion(base: u64, bytes: &[u8], fuse_pairs: bool) -> Self {
        let n_slots = bytes.len() / 4;
        let mut slots = Vec::with_capacity(n_slots);
        for i in 0..n_slots {
            let off = i * 4;
            let raw =
                u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
            slots.push(decode::decode(raw).ok());
        }
        quicken(&mut slots);
        if fuse_pairs {
            super_pair(&mut slots);
        }
        let written = (0..n_slots).map(|_| AtomicBool::new(false)).collect();
        Self {
            shared: Arc::new(Decoded {
                base,
                slots: slots.into_boxed_slice(),
                written,
            }),
            refreshed: BTreeMap::new(),
        }
    }

    /// Guest base address of the shadowed range.
    #[inline]
    pub fn base(&self) -> u64 {
        self.shared.base
    }

    /// Number of instruction slots.
    #[inline]
    pub fn len(&self) -> usize {
        self.shared.slots.len()
    }

    /// Whether the shadow has no slots.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.shared.slots.is_empty()
    }

    /// End address (exclusive) of the shadowed range.
    #[inline]
    pub fn end(&self) -> u64 {
        self.shared.base + (self.shared.slots.len() as u64) * 4
    }

    /// Whether `self` and `other` share one build-time decode, as a
    /// shadow and its clone do.
    pub fn shares_decode(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }

    /// Fetch the instruction at `pc`.
    ///
    /// `None` means the caller must fall back to raw fetch + decode:
    /// slot is stale, decode failed at build time, `pc` is out of
    /// range, or `pc` is misaligned.
    #[inline]
    pub fn get(&self, pc: u64) -> Option<PpuInstruction> {
        let idx = self.index(pc)?;
        if self.shared.written[idx].load(Ordering::Relaxed) {
            return self.refreshed_slot(idx);
        }
        self.shared.slots[idx]
    }

    /// Mark every slot overlapping `[addr, addr + len)` as stale.
    ///
    /// The range widens to both halves of each fused pair it touches.
    /// A stale `Consumed` half beside a fresh head lets the head's fused
    /// operation run for a rewritten second instruction. The runtime uses
    /// a raw fetch and decode for both halves until the caller calls
    /// [`refresh`](Self::refresh) on the new bytes.
    ///
    /// The slots also go stale in each sharing shadow that did not
    /// refresh them.
    pub fn invalidate_range(&mut self, addr: u64, len: u64) {
        if len == 0 {
            return;
        }
        let end = addr.saturating_add(len);
        let shadow_end = self.end();
        let base = self.base();
        if addr >= shadow_end || end <= base {
            return;
        }
        let clamp_lo = addr.max(base);
        let clamp_hi = end.min(shadow_end);
        let mut first_slot = ((clamp_lo - base) / 4) as usize;
        let mut last_slot = ((clamp_hi - base) as usize).div_ceil(4);
        last_slot = last_slot.min(self.len());

        // Super-pair partner widening. A super-pair has its head at
        // index k and a `Consumed` placeholder at k+1; the two slots
        // must transition to stale together or the fused dispatch and
        // the freshly-written byte at the partner slot disagree.
        if first_slot > 0 && matches!(self.content(first_slot), Some(PpuInstruction::Consumed)) {
            first_slot -= 1;
        }
        if last_slot < self.len()
            && matches!(self.content(last_slot), Some(PpuInstruction::Consumed))
        {
            last_slot += 1;
        }

        for flag in &self.shared.written[first_slot..last_slot] {
            flag.store(true, Ordering::Relaxed);
        }
        for (_, (_, stale)) in self.refreshed.range_mut(first_slot..last_slot) {
            *stale = true;
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
    /// separate dispatches until the next full shadow rebuild. When the
    /// slot is one half of a fused pair, the other half goes stale. Thus
    /// no fresh `Consumed` outlives its head, and no fresh head fuses a
    /// rewritten second instruction.
    ///
    /// The new decode belongs to this shadow alone. The slot goes stale
    /// in each sharing shadow that did not refresh it.
    pub fn refresh(&mut self, pc: u64, raw: u32) -> Option<Option<PpuInstruction>> {
        let idx = self.index(pc)?;
        if let Some(partner) = self.pair_partner(idx) {
            self.mark_stale(partner);
        }
        let insn = decode::decode(raw).ok();
        let quickened = insn.map(|i| quicken_insn(i).unwrap_or(i));
        self.shared.written[idx].store(true, Ordering::Relaxed);
        self.refreshed.insert(idx, (quickened, false));
        Some(quickened)
    }

    /// The slot index of `pc`, or `None` when `pc` is out of range or
    /// misaligned.
    #[inline]
    fn index(&self, pc: u64) -> Option<usize> {
        let byte_offset = pc.checked_sub(self.shared.base)?;
        if byte_offset & 3 != 0 {
            return None;
        }
        let idx = (byte_offset / 4) as usize;
        (idx < self.shared.slots.len()).then_some(idx)
    }

    /// The decode this shadow refreshed slot `idx` with, while it is
    /// fresh.
    #[cold]
    fn refreshed_slot(&self, idx: usize) -> Option<PpuInstruction> {
        match self.refreshed.get(&idx) {
            Some(&(insn, false)) => insn,
            _ => None,
        }
    }

    /// The decode slot `idx` holds for this shadow, fresh or stale.
    fn content(&self, idx: usize) -> Option<PpuInstruction> {
        match self.refreshed.get(&idx) {
            Some(&(insn, _)) => insn,
            None => self.shared.slots[idx],
        }
    }

    /// Marks slot `idx` stale here and in each sharing shadow that did not
    /// refresh it.
    fn mark_stale(&mut self, idx: usize) {
        match self.refreshed.get_mut(&idx) {
            Some((_, stale)) => *stale = true,
            None => self.shared.written[idx].store(true, Ordering::Relaxed),
        }
    }

    /// The other half of the fused pair slot `idx` belongs to, if any.
    fn pair_partner(&self, idx: usize) -> Option<usize> {
        let is_head = |i: usize| self.content(i).is_some_and(|insn| insn.is_super_pair());
        let is_consumed = |i: usize| matches!(self.content(i), Some(PpuInstruction::Consumed));
        if is_head(idx) && idx + 1 < self.len() && is_consumed(idx + 1) {
            return Some(idx + 1);
        }
        if is_consumed(idx) && idx > 0 && is_head(idx - 1) {
            return Some(idx - 1);
        }
        None
    }
}

/// Rewrite decoded slots in place into specialized instruction variants.
fn quicken(slots: &mut [Option<PpuInstruction>]) {
    for slot in slots.iter_mut() {
        if let Some(insn) = *slot {
            if let Some(quick) = quicken_insn(insn) {
                *slot = Some(quick);
            }
        }
    }
}

/// Fuse adjacent instruction pairs into superinstructions.
///
/// Run it after [`quicken`]: the fuser matches quickened forms such as
/// `Li`.
fn super_pair(slots: &mut [Option<PpuInstruction>]) {
    let n = slots.len();
    if n < 2 {
        return;
    }
    let mut i = 0;
    while i < n - 1 {
        let (a, b) = match (slots[i], slots[i + 1]) {
            (Some(a), Some(b)) => (a, b),
            _ => {
                i += 1;
                continue;
            }
        };
        if let Some(super_insn) = make_super_pair(a, b) {
            slots[i] = Some(super_insn);
            slots[i + 1] = Some(PpuInstruction::Consumed);
            // Skip `Consumed`, so the fuser never pairs it with slot i+2.
            i += 2;
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
impl PredecodedShadow {
    /// The decoded content and stale flag of slot `idx`.
    pub(super) fn slot(&self, idx: usize) -> Option<(Option<PpuInstruction>, bool)> {
        if idx >= self.len() {
            return None;
        }
        let stale = match self.refreshed.get(&idx) {
            Some(&(_, stale)) => stale,
            None => self.shared.written[idx].load(Ordering::Relaxed),
        };
        Some((self.content(idx), stale))
    }
}

#[cfg(test)]
#[path = "tests/model_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/model_proptests.rs"]
mod proptests;

#[cfg(test)]
#[path = "tests/refresh_pair_tests.rs"]
mod refresh_pair_tests;

#[cfg(test)]
#[path = "tests/shared_copy_tests.rs"]
mod shared_copy_tests;
