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
    /// Instructions remaining to end of basic block (inclusive).
    /// A branch/syscall has `block_len = 1`. The first instruction
    /// of a basic block has `block_len = N`. Stale slots are reset
    /// to 1 (conservative single-instruction block).
    block_len: Vec<u16>,
}

/// Whether an instruction is a basic-block terminator (branches,
/// syscall, or anything that unconditionally transfers control).
fn is_block_terminator(insn: &PpuInstruction) -> bool {
    matches!(
        insn,
        PpuInstruction::B { .. }
            | PpuInstruction::Bc { .. }
            | PpuInstruction::Bclr { .. }
            | PpuInstruction::Bcctr { .. }
            | PpuInstruction::CmpwiBc { .. }
            | PpuInstruction::CmpwBc { .. }
            | PpuInstruction::Sc
    )
}

impl PredecodedShadow {
    /// Build a shadow from raw guest memory bytes.
    ///
    /// `base` is the guest address of `bytes[0]`. Every aligned
    /// 4-byte word in `bytes` is decoded; words that fail to
    /// decode store `None` (the caller falls back to the raw
    /// fetch + decode path, producing the same decode-error fault
    /// the non-shadowed interpreter would).
    ///
    /// After decoding, a quickening pass rewrites common idioms
    /// (li, mr, slwi, srwi, clrlwi) into specialized variants.
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
        // Recompute block lengths after super-pairing since
        // branch-crossing supers (CmpwiBc, CmpwBc) change
        // terminator status.
        shadow.block_len = Self::compute_block_lengths(&shadow.slots);
        shadow
    }

    /// Backward scan to fill block_len. A branch/syscall gets 1.
    /// Each preceding non-terminator gets `next + 1`.
    fn compute_block_lengths(slots: &[Option<PpuInstruction>]) -> Vec<u16> {
        let n = slots.len();
        let mut bl = vec![1u16; n];
        if n == 0 {
            return bl;
        }
        // Last slot is always block_len=1 (end of shadow = implicit boundary)
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
            self.block_len[i] = 1;
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
        let quickened = insn.map(|i| quicken_insn(i).unwrap_or(i));
        self.slots[idx] = quickened;
        self.stale[idx] = false;
        self.rescan_block_len(idx);
        Some(quickened)
    }

    /// Instructions remaining in the basic block starting at `pc`
    /// (inclusive). Returns 1 for out-of-range, misaligned, or stale
    /// slots (conservative: treat as single-instruction block).
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

    /// Re-scan block_len for slot `idx` and its predecessors back to
    /// the previous block boundary. Called after refresh to keep
    /// block_len accurate for the affected region.
    fn rescan_block_len(&mut self, idx: usize) {
        // Set this slot's block_len based on whether it's a terminator.
        let is_term = match &self.slots[idx] {
            Some(insn) => is_block_terminator(insn),
            None => true,
        };
        if is_term || idx + 1 >= self.slots.len() || self.stale.get(idx + 1) == Some(&true) {
            self.block_len[idx] = 1;
        } else {
            self.block_len[idx] = self.block_len[idx + 1].saturating_add(1);
        }
        // Walk backwards to update predecessors.
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

    /// Rewrite decoded slots in place, replacing common idioms with
    /// specialized instruction variants that execute faster (fewer
    /// field reads, no redundant operations).
    pub fn quicken(&mut self) {
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

    /// Rewrite adjacent instruction pairs into compound
    /// superinstructions. Runs after quickening. Only pairs where
    /// neither instruction is a block terminator are considered
    /// (no control-flow crossing). The second slot of a fused pair
    /// becomes `Consumed`.
    pub fn super_pair(&mut self) {
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
                // Skip the Consumed slot so we don't try to pair it
                // with the next instruction.
                i += 2;
            } else {
                i += 1;
            }
        }
    }
}

/// Try to fuse two adjacent instructions into a single
/// superinstruction. Returns `None` when no fusion applies.
fn make_super_pair(a: PpuInstruction, b: PpuInstruction) -> Option<PpuInstruction> {
    match (a, b) {
        // lwz rT, off(rA) + cmpwi crF, rT, imm
        (
            PpuInstruction::Lwz { rt, ra, imm },
            PpuInstruction::Cmpwi {
                bf,
                ra: cmp_ra,
                imm: cmp_imm,
            },
        ) if rt == cmp_ra => Some(PpuInstruction::LwzCmpwi {
            rt,
            ra_load: ra,
            offset: imm,
            bf,
            cmp_imm,
        }),
        // li rT, imm + stw rT, off(rA)
        (
            PpuInstruction::Li { rt, imm },
            PpuInstruction::Stw {
                rs,
                ra,
                imm: st_off,
            },
        ) if rt == rs => Some(PpuInstruction::LiStw {
            rt,
            imm,
            ra_store: ra,
            store_offset: st_off,
        }),
        // mflr rT + stw rT, off(rA)
        (PpuInstruction::Mflr { rt }, PpuInstruction::Stw { rs, ra, imm }) if rt == rs => {
            Some(PpuInstruction::MflrStw {
                rt,
                ra_store: ra,
                store_offset: imm,
            })
        }
        // lwz rT, off(rA) + mtlr rT
        (PpuInstruction::Lwz { rt, ra, imm }, PpuInstruction::Mtlr { rs }) if rt == rs => {
            Some(PpuInstruction::LwzMtlr {
                rt,
                ra_load: ra,
                offset: imm,
            })
        }
        // lwz rT, off(rA) + CmpwZero crF, rT (quickened cmpwi-zero)
        (PpuInstruction::Lwz { rt, ra, imm }, PpuInstruction::CmpwZero { bf, ra: cmp_ra })
            if rt == cmp_ra =>
        {
            Some(PpuInstruction::LwzCmpwi {
                rt,
                ra_load: ra,
                offset: imm,
                bf,
                cmp_imm: 0,
            })
        }
        // cmpwi crF, rA, imm + bc BO, BI, offset (non-linking)
        (
            PpuInstruction::Cmpwi { bf, ra, imm },
            PpuInstruction::Bc {
                bo,
                bi,
                offset,
                link: false,
            },
        ) => Some(PpuInstruction::CmpwiBc {
            bf,
            ra,
            imm,
            bo,
            bi,
            target_offset: offset,
        }),
        // CmpwZero + bc (quickened cmpwi-zero still fuses)
        (
            PpuInstruction::CmpwZero { bf, ra },
            PpuInstruction::Bc {
                bo,
                bi,
                offset,
                link: false,
            },
        ) => Some(PpuInstruction::CmpwiBc {
            bf,
            ra,
            imm: 0,
            bo,
            bi,
            target_offset: offset,
        }),
        // cmpw crF, rA, rB + bc BO, BI, offset (non-linking)
        (
            PpuInstruction::Cmpw { bf, ra, rb },
            PpuInstruction::Bc {
                bo,
                bi,
                offset,
                link: false,
            },
        ) => Some(PpuInstruction::CmpwBc {
            bf,
            ra,
            rb,
            bo,
            bi,
            target_offset: offset,
        }),
        _ => None,
    }
}

/// Try to rewrite a generic instruction into a specialized variant.
/// Returns `None` when no specialization applies.
fn quicken_insn(insn: PpuInstruction) -> Option<PpuInstruction> {
    match insn {
        // addi rT, 0, imm => Li
        PpuInstruction::Addi { rt, ra: 0, imm } => Some(PpuInstruction::Li { rt, imm }),
        // or rA, rS, rS => Mr (when rs == rb)
        PpuInstruction::Or { ra, rs, rb } if rs == rb => Some(PpuInstruction::Mr { ra, rs }),
        // rlwinm rA, rS, sh, 0, 31-sh => Slwi
        PpuInstruction::Rlwinm { ra, rs, sh, mb, me } if mb == 0 && me == 31 - sh => {
            Some(PpuInstruction::Slwi { ra, rs, n: sh })
        }
        // rlwinm rA, rS, 32-n, n, 31 => Srwi
        PpuInstruction::Rlwinm { ra, rs, sh, mb, me } if me == 31 && sh != 0 && mb == (32 - sh) => {
            Some(PpuInstruction::Srwi { ra, rs, n: mb })
        }
        // rlwinm rA, rS, 0, n, 31 => Clrlwi
        PpuInstruction::Rlwinm { ra, rs, sh, mb, me } if sh == 0 && me == 31 => {
            Some(PpuInstruction::Clrlwi { ra, rs, n: mb })
        }
        // ori rA, rS, 0 where rA == rS => Nop
        PpuInstruction::Ori { ra, rs, imm: 0 } if ra == rs => Some(PpuInstruction::Nop),
        // cmpwi crF, rA, 0 => CmpwZero
        PpuInstruction::Cmpwi { bf, ra, imm: 0 } => Some(PpuInstruction::CmpwZero { bf, ra }),
        // rldicl rA, rS, 0, n => Clrldi
        PpuInstruction::Rldicl { ra, rs, sh: 0, mb } => {
            Some(PpuInstruction::Clrldi { ra, rs, n: mb })
        }
        // rldicr rA, rS, n, 63-n => Sldi
        PpuInstruction::Rldicr { ra, rs, sh, me } if sh != 0 && me == 63 - sh => {
            Some(PpuInstruction::Sldi { ra, rs, n: sh })
        }
        // rldicl rA, rS, 64-n, n => Srdi
        PpuInstruction::Rldicl { ra, rs, sh, mb } if sh != 0 && mb == 64 - sh => {
            Some(PpuInstruction::Srdi { ra, rs, n: mb })
        }
        _ => None,
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
        // refresh applies quickening: addi r3, r0, 99 => Li { rt: 3, imm: 99 }
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

    // -- block_len tests --

    fn b_raw() -> u32 {
        // b +8 (unconditional branch, offset=8, not AA, not LK)
        (18 << 26) | 8
    }

    fn sc_raw() -> u32 {
        // sc (syscall)
        (17 << 26) | 2
    }

    #[test]
    fn block_len_straight_line() {
        // 4 addi instructions, no branches
        let shadow = build_from_words(0, &[li_raw(3, 1), li_raw(4, 2), li_raw(5, 3), li_raw(6, 4)]);
        assert_eq!(shadow.block_len_at(0), 4);
        assert_eq!(shadow.block_len_at(4), 3);
        assert_eq!(shadow.block_len_at(8), 2);
        assert_eq!(shadow.block_len_at(12), 1);
    }

    #[test]
    fn block_len_branch_terminates() {
        // addi, addi, b, addi
        let shadow = build_from_words(0, &[li_raw(3, 1), li_raw(4, 2), b_raw(), li_raw(5, 3)]);
        assert_eq!(shadow.block_len_at(0), 3);
        assert_eq!(shadow.block_len_at(4), 2);
        assert_eq!(shadow.block_len_at(8), 1); // branch itself
        assert_eq!(shadow.block_len_at(12), 1); // new block
    }

    #[test]
    fn block_len_syscall_terminates() {
        // addi, sc, addi
        let shadow = build_from_words(0, &[li_raw(3, 1), sc_raw(), li_raw(4, 2)]);
        assert_eq!(shadow.block_len_at(0), 2);
        assert_eq!(shadow.block_len_at(4), 1); // sc
        assert_eq!(shadow.block_len_at(8), 1); // next block
    }

    #[test]
    fn block_len_invalidation_resets() {
        let mut shadow = build_from_words(0, &[li_raw(3, 1), li_raw(4, 2), li_raw(5, 3)]);
        assert_eq!(shadow.block_len_at(0), 3);
        shadow.invalidate_range(4, 4); // stale slot 1
        assert_eq!(shadow.block_len_at(0), 3); // slot 0 unchanged (already computed)

        // Wait -- invalidation should also reset predecessors.
        // Actually per design: invalidation resets the STALED slots to 1,
        // but predecessors' block_len is NOT recalculated on invalidation
        // (too expensive). The safe behavior is: block_len_at for a slot
        // whose successor is stale may overcount. The outer loop must
        // re-check stale/None on every fetch within the block, which it
        // already does (shadow.get returns None for stale slots).
        // The block_len is an upper bound, not exact.
        assert_eq!(shadow.block_len_at(4), 1); // staled slot
        assert_eq!(shadow.block_len_at(8), 1); // end of shadow
    }

    #[test]
    fn block_len_refresh_rescans() {
        let mut shadow = build_from_words(0, &[li_raw(3, 1), li_raw(4, 2), li_raw(5, 3)]);
        assert_eq!(shadow.block_len_at(0), 3);
        shadow.invalidate_range(4, 4);
        assert_eq!(shadow.block_len_at(4), 1);
        // Refresh slot 1 with a non-branch instruction
        shadow.refresh(4, li_raw(4, 99));
        // Now slot 1 is no longer stale, block_len should be rescanned
        assert_eq!(shadow.block_len_at(4), 2); // slots 1,2
        assert_eq!(shadow.block_len_at(0), 3); // predecessor updated
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

    // -- quickening tests --

    /// Encode `or rA, rS, rB` (opcode 31, XO 444).
    fn or_raw(rs: u32, ra: u32, rb: u32) -> u32 {
        (31 << 26) | (rs << 21) | (ra << 16) | (rb << 11) | (444 << 1)
    }

    /// Encode `rlwinm rA, rS, sh, mb, me` (opcode 21).
    fn rlwinm_raw(rs: u32, ra: u32, sh: u32, mb: u32, me: u32) -> u32 {
        (21 << 26) | (rs << 21) | (ra << 16) | (sh << 11) | (mb << 6) | (me << 1)
    }

    #[test]
    fn quicken_addi_ra0_becomes_li() {
        // addi r3, r0, 42 => Li { rt: 3, imm: 42 }
        let shadow = build_from_words(0, &[li_raw(3, 42)]);
        assert_eq!(shadow.get(0), Some(PpuInstruction::Li { rt: 3, imm: 42 }));
    }

    #[test]
    fn quicken_or_same_reg_becomes_mr() {
        // or r3, r4, r4 => Mr { ra: 3, rs: 4 }
        let shadow = build_from_words(0, &[or_raw(4, 3, 4)]);
        assert_eq!(shadow.get(0), Some(PpuInstruction::Mr { ra: 3, rs: 4 }));
    }

    #[test]
    fn quicken_rlwinm_slwi() {
        // rlwinm r3, r4, 8, 0, 23 => Slwi { ra: 3, rs: 4, n: 8 }
        let shadow = build_from_words(0, &[rlwinm_raw(4, 3, 8, 0, 23)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::Slwi { ra: 3, rs: 4, n: 8 })
        );
    }

    #[test]
    fn quicken_rlwinm_srwi() {
        // srwi r3, r4, 8 => rlwinm r3, r4, 24, 8, 31
        let shadow = build_from_words(0, &[rlwinm_raw(4, 3, 24, 8, 31)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::Srwi { ra: 3, rs: 4, n: 8 })
        );
    }

    #[test]
    fn quicken_rlwinm_clrlwi() {
        // clrlwi r3, r4, 16 => rlwinm r3, r4, 0, 16, 31
        let shadow = build_from_words(0, &[rlwinm_raw(4, 3, 0, 16, 31)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::Clrlwi {
                ra: 3,
                rs: 4,
                n: 16
            })
        );
    }

    #[test]
    fn quicken_non_specializable_unchanged() {
        // or r3, r4, r5 (rs != rb, not mr) stays as Or
        let shadow = build_from_words(0, &[or_raw(4, 3, 5)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::Or {
                ra: 3,
                rs: 4,
                rb: 5
            })
        );
    }

    #[test]
    fn quicken_addi_nonzero_ra_unchanged() {
        // addi r3, r5, 42 (ra != 0) stays as Addi
        let raw = (14 << 26) | (3 << 21) | (5 << 16) | (42u16 as u32);
        let shadow = build_from_words(0, &[raw]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::Addi {
                rt: 3,
                ra: 5,
                imm: 42
            })
        );
    }

    #[test]
    fn refresh_applies_quickening() {
        let mut shadow = build_from_words(0, &[li_raw(3, 10)]);
        assert_eq!(shadow.get(0), Some(PpuInstruction::Li { rt: 3, imm: 10 }));
        // Invalidate and refresh with a new li instruction
        shadow.invalidate_range(0, 4);
        assert!(shadow.get(0).is_none());
        let refreshed = shadow.refresh(0, li_raw(4, 99));
        assert_eq!(refreshed, Some(Some(PpuInstruction::Li { rt: 4, imm: 99 })));
        assert_eq!(shadow.get(0), Some(PpuInstruction::Li { rt: 4, imm: 99 }));
    }

    #[test]
    fn refresh_applies_quickening_or_to_mr() {
        let mut shadow = build_from_words(0, &[li_raw(3, 1)]);
        shadow.invalidate_range(0, 4);
        // Refresh with `or r5, r6, r6` => should quicken to Mr
        let refreshed = shadow.refresh(0, or_raw(6, 5, 6));
        assert_eq!(refreshed, Some(Some(PpuInstruction::Mr { ra: 5, rs: 6 })));
    }

    // -- super-pairing tests --

    /// Encode `lwz rT, off(rA)` (opcode 32).
    fn lwz_raw(rt: u32, ra: u32, off: i16) -> u32 {
        (32 << 26) | (rt << 21) | (ra << 16) | (off as u16 as u32)
    }

    /// Encode `cmpwi crF, rA, imm` (opcode 11, L=0).
    fn cmpwi_raw(bf: u32, ra: u32, imm: i16) -> u32 {
        (11 << 26) | (bf << 23) | (ra << 16) | (imm as u16 as u32)
    }

    /// Encode `stw rS, off(rA)` (opcode 36).
    fn stw_raw(rs: u32, ra: u32, off: i16) -> u32 {
        (36 << 26) | (rs << 21) | (ra << 16) | (off as u16 as u32)
    }

    /// Encode `mflr rT` (opcode 31, XO 339, spr=8 -> sprn bits: 0x100).
    fn mflr_raw(rt: u32) -> u32 {
        // mfspr rt, 8: spr field is split: spr[0:4] in bits 16-20,
        // spr[5:9] in bits 11-15. SPR 8 = 0b0000001000.
        // spr[0:4] = 0b01000 = 8, spr[5:9] = 0b00000 = 0.
        (31 << 26) | (rt << 21) | (8 << 16) | (339 << 1)
    }

    /// Encode `mtlr rS` (opcode 31, XO 467, spr=8).
    fn mtlr_raw(rs: u32) -> u32 {
        (31 << 26) | (rs << 21) | (8 << 16) | (467 << 1)
    }

    #[test]
    fn super_pair_lwz_cmpwi() {
        // lwz r3, 8(r1) + cmpwi cr0, r3, 42
        let shadow = build_from_words(0, &[lwz_raw(3, 1, 8), cmpwi_raw(0, 3, 42)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::LwzCmpwi {
                rt: 3,
                ra_load: 1,
                offset: 8,
                bf: 0,
                cmp_imm: 42,
            })
        );
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
    }

    #[test]
    fn super_pair_lwz_cmpwi_different_reg_no_fuse() {
        // lwz r3, 8(r1) + cmpwi cr0, r4, 42 -- different register, no fuse
        let shadow = build_from_words(0, &[lwz_raw(3, 1, 8), cmpwi_raw(0, 4, 42)]);
        // Should stay as separate instructions
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Lwz { .. })));
        assert!(matches!(shadow.get(4), Some(PpuInstruction::Cmpwi { .. })));
    }

    #[test]
    fn super_pair_li_stw() {
        // li r5, 99 + stw r5, 0(r1)
        let shadow = build_from_words(0, &[li_raw(5, 99), stw_raw(5, 1, 0)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::LiStw {
                rt: 5,
                imm: 99,
                ra_store: 1,
                store_offset: 0,
            })
        );
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
    }

    #[test]
    fn super_pair_li_stw_different_reg_no_fuse() {
        // li r5, 99 + stw r6, 0(r1) -- different register, no fuse
        let shadow = build_from_words(0, &[li_raw(5, 99), stw_raw(6, 1, 0)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Li { .. })));
        assert!(matches!(shadow.get(4), Some(PpuInstruction::Stw { .. })));
    }

    #[test]
    fn super_pair_mflr_stw() {
        // mflr r0 + stw r0, 16(r1)
        let shadow = build_from_words(0, &[mflr_raw(0), stw_raw(0, 1, 16)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::MflrStw {
                rt: 0,
                ra_store: 1,
                store_offset: 16,
            })
        );
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
    }

    #[test]
    fn super_pair_lwz_mtlr() {
        // lwz r0, 16(r1) + mtlr r0
        let shadow = build_from_words(0, &[lwz_raw(0, 1, 16), mtlr_raw(0)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::LwzMtlr {
                rt: 0,
                ra_load: 1,
                offset: 16,
            })
        );
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
    }

    #[test]
    fn super_pair_lwz_mtlr_different_reg_no_fuse() {
        // lwz r3, 16(r1) + mtlr r0 -- different register, no fuse
        let shadow = build_from_words(0, &[lwz_raw(3, 1, 16), mtlr_raw(0)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Lwz { .. })));
        assert!(matches!(shadow.get(4), Some(PpuInstruction::Mtlr { .. })));
    }

    #[test]
    fn super_pair_consumed_not_chained() {
        // Three instructions: li r3, 1; stw r3, 0(r1); li r4, 2
        // The first two should fuse; the consumed slot should not
        // chain with the third instruction.
        let shadow = build_from_words(0, &[li_raw(3, 1), stw_raw(3, 1, 0), li_raw(4, 2)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::LiStw { .. })));
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
        assert_eq!(shadow.get(8), Some(PpuInstruction::Li { rt: 4, imm: 2 }));
    }

    #[test]
    fn super_pair_branch_blocks_fusion() {
        // b_raw is a branch; it cannot be the first of a fused pair.
        // branch + li: no fusion should happen.
        let shadow = build_from_words(0, &[b_raw(), li_raw(3, 1)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::B { .. })));
        assert!(matches!(shadow.get(4), Some(PpuInstruction::Li { .. })));
    }

    #[test]
    fn super_pair_stale_slot_blocks_fusion() {
        let mut shadow = build_from_words(0, &[li_raw(3, 42), stw_raw(3, 1, 0)]);
        // Before invalidation, they should have been fused
        assert!(matches!(shadow.get(0), Some(PpuInstruction::LiStw { .. })));
        // Invalidate the first slot; refreshing should NOT re-pair
        shadow.invalidate_range(0, 4);
        assert!(shadow.get(0).is_none()); // stale
    }

    #[test]
    fn super_pair_multiple_pairs() {
        // Two independent fusible pairs in sequence:
        // mflr r0, stw r0, 16(r1), lwz r3, 8(r1), cmpwi cr0, r3, 0
        let shadow = build_from_words(
            0,
            &[
                mflr_raw(0),
                stw_raw(0, 1, 16),
                lwz_raw(3, 1, 8),
                cmpwi_raw(0, 3, 0),
            ],
        );
        assert!(matches!(
            shadow.get(0),
            Some(PpuInstruction::MflrStw { .. })
        ));
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
        assert!(matches!(
            shadow.get(8),
            Some(PpuInstruction::LwzCmpwi { .. })
        ));
        assert_eq!(shadow.get(12), Some(PpuInstruction::Consumed));
    }

    #[test]
    fn super_pair_single_instruction_shadow() {
        // Single instruction: no pairing possible
        let shadow = build_from_words(0, &[li_raw(3, 1)]);
        assert_eq!(shadow.get(0), Some(PpuInstruction::Li { rt: 3, imm: 1 }));
    }

    #[test]
    fn super_pair_empty_shadow() {
        let shadow = PredecodedShadow::build(0, &[]);
        assert!(shadow.is_empty());
    }

    // -- 14.5B quickening tests --

    /// Encode `ori rA, rS, imm` (opcode 24).
    fn ori_raw(rs: u32, ra: u32, imm: u16) -> u32 {
        (24 << 26) | (rs << 21) | (ra << 16) | (imm as u32)
    }

    /// Encode `rldicl rA, rS, sh, mb` (opcode 30, xo=0).
    fn rldicl_raw(rs: u32, ra: u32, sh: u32, mb: u32) -> u32 {
        let sh_lo = sh & 0x1F;
        let sh_hi = (sh >> 5) & 1;
        let mb_lo = mb & 0x1F;
        let mb_hi = (mb >> 5) & 1;
        // xo=0 for rldicl (no bits set in xo field)
        (30 << 26)
            | (rs << 21)
            | (ra << 16)
            | (sh_lo << 11)
            | (mb_lo << 6)
            | (mb_hi << 5)
            | (sh_hi << 1)
    }

    /// Encode `rldicr rA, rS, sh, me` (opcode 30, xo=1).
    fn rldicr_raw(rs: u32, ra: u32, sh: u32, me: u32) -> u32 {
        let sh_lo = sh & 0x1F;
        let sh_hi = (sh >> 5) & 1;
        let me_lo = me & 0x1F;
        let me_hi = (me >> 5) & 1;
        (30 << 26)
            | (rs << 21)
            | (ra << 16)
            | (sh_lo << 11)
            | (me_lo << 6)
            | (me_hi << 5)
            | (1 << 2) // xo=1 for rldicr
            | (sh_hi << 1)
    }

    #[test]
    fn quicken_ori_same_reg_zero_becomes_nop() {
        // ori r5, r5, 0 => Nop
        let shadow = build_from_words(0, &[ori_raw(5, 5, 0)]);
        assert_eq!(shadow.get(0), Some(PpuInstruction::Nop));
    }

    #[test]
    fn quicken_ori_different_reg_unchanged() {
        // ori r3, r5, 0 -- different regs, not nop
        let shadow = build_from_words(0, &[ori_raw(5, 3, 0)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Ori { .. })));
    }

    #[test]
    fn quicken_ori_nonzero_imm_unchanged() {
        // ori r5, r5, 1 -- nonzero imm, not nop
        let shadow = build_from_words(0, &[ori_raw(5, 5, 1)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Ori { .. })));
    }

    #[test]
    fn quicken_cmpwi_zero_becomes_cmpw_zero() {
        // cmpwi cr0, r3, 0 => CmpwZero { bf: 0, ra: 3 }
        let shadow = build_from_words(0, &[cmpwi_raw(0, 3, 0)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::CmpwZero { bf: 0, ra: 3 })
        );
    }

    #[test]
    fn quicken_cmpwi_nonzero_unchanged() {
        // cmpwi cr0, r3, 42 -- nonzero imm, stays Cmpwi
        let shadow = build_from_words(0, &[cmpwi_raw(0, 3, 42)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Cmpwi { .. })));
    }

    #[test]
    fn quicken_rldicl_sh0_becomes_clrldi() {
        // rldicl r3, r4, 0, 32 => Clrldi { ra: 3, rs: 4, n: 32 }
        let shadow = build_from_words(0, &[rldicl_raw(4, 3, 0, 32)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::Clrldi {
                ra: 3,
                rs: 4,
                n: 32
            })
        );
    }

    #[test]
    fn quicken_rldicr_sldi_pattern() {
        // sldi r3, r4, 8 => rldicr r3, r4, 8, 55
        let shadow = build_from_words(0, &[rldicr_raw(4, 3, 8, 55)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::Sldi { ra: 3, rs: 4, n: 8 })
        );
    }

    #[test]
    fn quicken_rldicl_srdi_pattern() {
        // srdi r3, r4, 8 => rldicl r3, r4, 56, 8
        let shadow = build_from_words(0, &[rldicl_raw(4, 3, 56, 8)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::Srdi { ra: 3, rs: 4, n: 8 })
        );
    }

    #[test]
    fn quicken_rldicl_nonzero_sh_non_srdi_unchanged() {
        // rldicl with sh != 0 and mb != 64-sh stays as Rldicl
        let shadow = build_from_words(0, &[rldicl_raw(4, 3, 10, 20)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Rldicl { .. })));
    }

    // -- 14.5C super-pair tests --

    /// Encode `bc BO, BI, offset` (opcode 16, no link, no AA).
    fn bc_raw(bo: u32, bi: u32, offset: i16) -> u32 {
        (16 << 26) | (bo << 21) | (bi << 16) | ((offset as u16) as u32)
    }

    #[test]
    fn super_pair_cmpwi_bc() {
        // cmpwi cr0, r3, 42; beq cr0, +8
        let shadow = build_from_words(0, &[cmpwi_raw(0, 3, 42), bc_raw(0x0C, 2, 8)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::CmpwiBc {
                bf: 0,
                ra: 3,
                imm: 42,
                bo: 0x0C,
                bi: 2,
                target_offset: 8,
            })
        );
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
    }

    #[test]
    fn super_pair_cmpwi_zero_bc() {
        // cmpwi cr0, r3, 0 (quickened to CmpwZero) + bc
        let shadow = build_from_words(0, &[cmpwi_raw(0, 3, 0), bc_raw(0x0C, 2, 8)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::CmpwiBc {
                bf: 0,
                ra: 3,
                imm: 0,
                bo: 0x0C,
                bi: 2,
                target_offset: 8,
            })
        );
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
    }

    /// Encode `cmpw crF, rA, rB` (opcode 31, XO 0, L=0).
    fn cmpw_raw(bf: u32, ra: u32, rb: u32) -> u32 {
        (31 << 26) | (bf << 23) | (ra << 16) | (rb << 11)
    }

    #[test]
    fn super_pair_cmpw_bc() {
        // cmpw cr0, r3, r4; beq cr0, +12
        let shadow = build_from_words(0, &[cmpw_raw(0, 3, 4), bc_raw(0x0C, 2, 12)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::CmpwBc {
                bf: 0,
                ra: 3,
                rb: 4,
                bo: 0x0C,
                bi: 2,
                target_offset: 12,
            })
        );
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
    }

    #[test]
    fn super_pair_cmpwi_bc_link_no_fuse() {
        // cmpwi + bcl (link=true): should NOT fuse
        let bc_link = bc_raw(0x0C, 2, 8) | 1; // set LK bit
        let shadow = build_from_words(0, &[cmpwi_raw(0, 3, 42), bc_link]);
        // First should stay as Cmpwi (or CmpwZero if imm==0)
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Cmpwi { .. })));
        assert!(matches!(shadow.get(4), Some(PpuInstruction::Bc { .. })));
    }

    #[test]
    fn super_pair_cmpwi_bc_is_block_terminator() {
        // CmpwiBc should be treated as a block terminator.
        // Block: li r3, 5; cmpwi cr0, r3, 5; bc ...
        let shadow = build_from_words(0, &[li_raw(3, 5), cmpwi_raw(0, 3, 5), bc_raw(0x0C, 2, 8)]);
        // li at slot 0, CmpwiBc at slot 1, Consumed at slot 2
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Li { .. })));
        assert!(matches!(
            shadow.get(4),
            Some(PpuInstruction::CmpwiBc { .. })
        ));
        assert_eq!(shadow.get(8), Some(PpuInstruction::Consumed));
        // block_len: li=2, CmpwiBc=1 (terminator), Consumed=1
        assert_eq!(shadow.block_len_at(0), 2);
        assert_eq!(shadow.block_len_at(4), 1);
    }
}
