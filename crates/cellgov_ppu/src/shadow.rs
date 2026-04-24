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

use crate::decode;
use crate::instruction::PpuInstruction;

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
        let first_slot = ((clamp_lo - self.base) / 4) as usize;
        let last_slot = ((clamp_hi - self.base) as usize).div_ceil(4);
        let last_slot = last_slot.min(self.slots.len());
        for i in first_slot..last_slot {
            self.stale[i] = true;
            self.block_len[i] = 1;
        }
    }

    /// Re-decode a single slot and clear its stale bit.
    ///
    /// Outer `None` means `pc` is out of range or misaligned.
    /// `Some(None)` means decode failed (same slot state as the
    /// build-time decode-error path). Quickening is re-applied;
    /// super-pairing is not.
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
    /// variants. See [`quicken_insn`] for the idioms recognized.
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

    /// Fuse adjacent instruction pairs into superinstructions.
    ///
    /// Runs after [`quicken`](Self::quicken); the second slot of a
    /// fused pair is replaced with `Consumed`. Pairs that cross a
    /// block boundary (first operand is a terminator) are skipped,
    /// but fusions that *produce* a terminator (CmpwiBc/CmpwBc) are
    /// allowed -- callers must recompute `block_len` after this runs.
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
                // Skip Consumed so it is not fused with slot i+2.
                i += 2;
            } else {
                i += 1;
            }
        }
    }
}

/// Fuse two adjacent instructions, or `None` if no rule applies.
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
        // mflr rT + std rT, off(rA) (PPC64 prologue)
        (PpuInstruction::Mflr { rt }, PpuInstruction::Std { rs, ra, imm }) if rt == rs => {
            Some(PpuInstruction::MflrStd {
                rt,
                ra_store: ra,
                store_offset: imm,
            })
        }
        // ld rT, off(rA) + mtlr rT (PPC64 epilogue)
        (PpuInstruction::Ld { rt, ra, imm }, PpuInstruction::Mtlr { rs }) if rt == rs => {
            Some(PpuInstruction::LdMtlr {
                rt,
                ra_load: ra,
                offset: imm,
            })
        }
        // std rS1, off1(rA) + std rS2, off2(rA) where off2 = off1 + 8
        (
            PpuInstruction::Std {
                rs: rs1,
                ra: ra1,
                imm: off1,
            },
            PpuInstruction::Std {
                rs: rs2,
                ra: ra2,
                imm: off2,
            },
        ) if ra1 == ra2 && off2 == off1.wrapping_add(8) => Some(PpuInstruction::StdStd {
            rs1,
            rs2,
            ra: ra1,
            offset1: off1,
        }),
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

/// Rewrite one decoded instruction into a specialized variant, or
/// `None` if no rule applies.
fn quicken_insn(insn: PpuInstruction) -> Option<PpuInstruction> {
    match insn {
        // addi rT, 0, imm => Li
        PpuInstruction::Addi { rt, ra: 0, imm } => Some(PpuInstruction::Li { rt, imm }),
        // or rA, rS, rS => Mr (only when Rc=0; or. must keep CR0 update).
        PpuInstruction::Or {
            ra,
            rs,
            rb,
            rc: false,
        } if rs == rb => Some(PpuInstruction::Mr { ra, rs }),
        // rlwinm rA, rS, sh, 0, 31-sh => Slwi
        PpuInstruction::Rlwinm {
            ra,
            rs,
            sh,
            mb,
            me,
            rc: false,
        } if mb == 0 && me == 31 - sh => Some(PpuInstruction::Slwi { ra, rs, n: sh }),
        // rlwinm rA, rS, 32-n, n, 31 => Srwi
        PpuInstruction::Rlwinm {
            ra,
            rs,
            sh,
            mb,
            me,
            rc: false,
        } if me == 31 && sh != 0 && mb == (32 - sh) => Some(PpuInstruction::Srwi { ra, rs, n: mb }),
        // rlwinm rA, rS, 0, n, 31 => Clrlwi
        PpuInstruction::Rlwinm {
            ra,
            rs,
            sh,
            mb,
            me,
            rc: false,
        } if sh == 0 && me == 31 => Some(PpuInstruction::Clrlwi { ra, rs, n: mb }),
        // ori rA, rA, 0 => Nop
        PpuInstruction::Ori { ra, rs, imm: 0 } if ra == rs => Some(PpuInstruction::Nop),
        // cmpwi crF, rA, 0 => CmpwZero
        PpuInstruction::Cmpwi { bf, ra, imm: 0 } => Some(PpuInstruction::CmpwZero { bf, ra }),
        // rldicl rA, rS, 0, n => Clrldi
        PpuInstruction::Rldicl {
            ra,
            rs,
            sh: 0,
            mb,
            rc: false,
        } => Some(PpuInstruction::Clrldi { ra, rs, n: mb }),
        // rldicr rA, rS, n, 63-n => Sldi
        PpuInstruction::Rldicr {
            ra,
            rs,
            sh,
            me,
            rc: false,
        } if sh != 0 && me == 63 - sh => Some(PpuInstruction::Sldi { ra, rs, n: sh }),
        // rldicl rA, rS, 64-n, n => Srdi
        PpuInstruction::Rldicl {
            ra,
            rs,
            sh,
            mb,
            rc: false,
        } if sh != 0 && mb == 64 - sh => Some(PpuInstruction::Srdi { ra, rs, n: mb }),
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
        let shadow = build_from_words(0, &[li_raw(3, 1), li_raw(4, 2), li_raw(5, 3), li_raw(6, 4)]);
        assert_eq!(shadow.block_len_at(0), 4);
        assert_eq!(shadow.block_len_at(4), 3);
        assert_eq!(shadow.block_len_at(8), 2);
        assert_eq!(shadow.block_len_at(12), 1);
    }

    #[test]
    fn block_len_branch_terminates() {
        let shadow = build_from_words(0, &[li_raw(3, 1), li_raw(4, 2), b_raw(), li_raw(5, 3)]);
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
                rb: 5,
                rc: false,
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

    /// Encode `ld rT, off(rA)` (opcode 58, DS-form with sub=0).
    fn ld_raw(rt: u32, ra: u32, off: i16) -> u32 {
        (58 << 26) | (rt << 21) | (ra << 16) | ((off as u16 as u32) & 0xFFFC)
    }

    /// Encode `std rS, off(rA)` (opcode 62, DS-form with sub=0).
    fn std_raw(rs: u32, ra: u32, off: i16) -> u32 {
        (62 << 26) | (rs << 21) | (ra << 16) | ((off as u16 as u32) & 0xFFFC)
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
    fn super_pair_mflr_std() {
        let shadow = build_from_words(0, &[mflr_raw(0), std_raw(0, 1, 16)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::MflrStd {
                rt: 0,
                ra_store: 1,
                store_offset: 16,
            })
        );
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
    }

    #[test]
    fn super_pair_ld_mtlr() {
        let shadow = build_from_words(0, &[ld_raw(0, 1, 16), mtlr_raw(0)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::LdMtlr {
                rt: 0,
                ra_load: 1,
                offset: 16,
            })
        );
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
    }

    #[test]
    fn super_pair_std_std_adjacent() {
        // std r3, 0(r1) + std r4, 8(r1): fuse (same base, off2 = off1+8).
        let shadow = build_from_words(0, &[std_raw(3, 1, 0), std_raw(4, 1, 8)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::StdStd {
                rs1: 3,
                rs2: 4,
                ra: 1,
                offset1: 0,
            })
        );
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
    }

    #[test]
    fn super_pair_std_std_nonadjacent_no_fuse() {
        // std r3, 0(r1) + std r4, 16(r1): offset gap != 8, no fuse.
        let shadow = build_from_words(0, &[std_raw(3, 1, 0), std_raw(4, 1, 16)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Std { .. })));
        assert!(matches!(shadow.get(4), Some(PpuInstruction::Std { .. })));
    }

    #[test]
    fn super_pair_std_std_different_base_no_fuse() {
        // std r3, 0(r1) + std r4, 8(r2): different base, no fuse.
        let shadow = build_from_words(0, &[std_raw(3, 1, 0), std_raw(4, 2, 8)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Std { .. })));
        assert!(matches!(shadow.get(4), Some(PpuInstruction::Std { .. })));
    }

    #[test]
    fn super_pair_consumed_not_chained() {
        // Consumed must not pair forward with slot i+2.
        let shadow = build_from_words(0, &[li_raw(3, 1), stw_raw(3, 1, 0), li_raw(4, 2)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::LiStw { .. })));
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
        assert_eq!(shadow.get(8), Some(PpuInstruction::Li { rt: 4, imm: 2 }));
    }

    #[test]
    fn super_pair_branch_blocks_fusion() {
        let shadow = build_from_words(0, &[b_raw(), li_raw(3, 1)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::B { .. })));
        assert!(matches!(shadow.get(4), Some(PpuInstruction::Li { .. })));
    }

    #[test]
    fn super_pair_stale_slot_blocks_fusion() {
        let mut shadow = build_from_words(0, &[li_raw(3, 42), stw_raw(3, 1, 0)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::LiStw { .. })));
        shadow.invalidate_range(0, 4);
        assert!(shadow.get(0).is_none());
    }

    #[test]
    fn super_pair_multiple_pairs() {
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
        let bc_link = bc_raw(0x0C, 2, 8) | 1; // LK bit
        let shadow = build_from_words(0, &[cmpwi_raw(0, 3, 42), bc_link]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Cmpwi { .. })));
        assert!(matches!(shadow.get(4), Some(PpuInstruction::Bc { .. })));
    }

    #[test]
    fn super_pair_cmpwi_bc_is_block_terminator() {
        let shadow = build_from_words(0, &[li_raw(3, 5), cmpwi_raw(0, 3, 5), bc_raw(0x0C, 2, 8)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Li { .. })));
        assert!(matches!(
            shadow.get(4),
            Some(PpuInstruction::CmpwiBc { .. })
        ));
        assert_eq!(shadow.get(8), Some(PpuInstruction::Consumed));
        assert_eq!(shadow.block_len_at(0), 2);
        assert_eq!(shadow.block_len_at(4), 1);
    }
}
