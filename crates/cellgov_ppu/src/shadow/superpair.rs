//! Super-pairing pass: fuse adjacent decoded instructions into a
//! single super-instruction variant when an idiom matches. Pure
//! function; the shadow's `super_pair` method walks slot pairs and
//! applies this, replacing the second slot with `Consumed`.
//!
//! [ErtlGregg2003 p:20 s:6.3] super-instruction fusion eliminates one dispatch per pair.
//! [ErtlGregg2003 p:19 s:5.2.7] empirical effectiveness of super-instructions.
//!
//! Pair fusions that produce a block-terminator (`CmpwiBc`,
//! `CmpwBc`) require the caller to recompute `block_len` after this
//! pass.
//!
//! # Pairing priority
//!
//! The walk is left-to-right and first-pair-wins: if slot k can fuse
//! with slot k+1, the fusion takes slot k+2 out of any subsequent
//! pairing decision involving k+1. The common three-instruction
//! prologue `lwz; cmpwi; bc` therefore fuses to `LwzCmpwi + bc`
//! rather than `lwz + CmpwiBc`. Both fusions remove one dispatch;
//! the choice between them would require profiling data this pass
//! does not have.
//!
//! # Variants not fused
//!
//! `cmplwi` (unsigned immediate compare) and `cmplw` (unsigned
//! register compare) before a non-linking `bc` would mirror the
//! `Cmpwi`/`Cmpw` paths but no fused variant exists yet -- the
//! signed compares were profiled first. Adding `CmplwiBc` /
//! `CmplwBc` would be a mechanical extension once profiling shows
//! the unsigned forms above the project's >1% threshold.

use crate::instruction::PpuInstruction;

/// Fuse two adjacent instructions, or `None` if no rule applies.
pub(super) fn make_super_pair(a: PpuInstruction, b: PpuInstruction) -> Option<PpuInstruction> {
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
        // std rS1, off1(rA) + std rS2, off2(rA) where off2 = off1 + 8.
        // checked_add (not wrapping_add) rejects a near-i16::MAX off1
        // whose +8 would wrap to a negative offset and falsely match a
        // distant store -- the two stores would not actually be
        // accessing adjacent doublewords.
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
        ) if ra1 == ra2 && off1.checked_add(8) == Some(off2) => Some(PpuInstruction::StdStd {
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
                aa: false,
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
                aa: false,
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
                aa: false,
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

#[cfg(test)]
#[path = "tests/superpair_tests.rs"]
mod tests;
