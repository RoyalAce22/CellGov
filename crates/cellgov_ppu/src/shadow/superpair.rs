//! Super-pairing pass: fuse adjacent decoded instructions into a
//! single super-instruction variant when an idiom matches. Pure
//! function; the shadow's `super_pair` method walks slot pairs and
//! applies this, replacing the second slot with `Consumed`.
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
mod tests {
    use super::super::test_support::{
        b_raw, bc_raw, build_from_words, cmpw_raw, cmpwi_raw, ld_raw, li_raw, lwz_raw, mflr_raw,
        mtlr_raw, std_raw, stw_raw,
    };
    use crate::instruction::PpuInstruction;

    #[test]
    fn super_pair_lwz_cmpwi() {
        // lwz r3, 8(r1) + cmpwi cr0, r3, 42 -> LwzCmpwi
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
        let shadow = build_from_words(0, &[lwz_raw(3, 1, 8), cmpwi_raw(0, 4, 42)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Lwz { .. })));
        assert!(matches!(shadow.get(4), Some(PpuInstruction::Cmpwi { .. })));
    }

    #[test]
    fn super_pair_li_stw() {
        // li r3, 99 + stw r3, 0(r1) -> LiStw
        let shadow = build_from_words(0, &[li_raw(3, 99), stw_raw(3, 1, 0)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::LiStw {
                rt: 3,
                imm: 99,
                ra_store: 1,
                store_offset: 0,
            })
        );
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
    }

    #[test]
    fn super_pair_li_stw_different_reg_no_fuse() {
        let shadow = build_from_words(0, &[li_raw(3, 99), stw_raw(4, 1, 0)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Li { .. })));
        assert!(matches!(shadow.get(4), Some(PpuInstruction::Stw { .. })));
    }

    #[test]
    fn super_pair_mflr_stw() {
        // mflr r0 + stw r0, 4(r1) -> MflrStw
        let shadow = build_from_words(0, &[mflr_raw(0), stw_raw(0, 1, 4)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::MflrStw {
                rt: 0,
                ra_store: 1,
                store_offset: 4,
            })
        );
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
    }

    #[test]
    fn super_pair_lwz_mtlr() {
        // lwz r0, 4(r1) + mtlr r0 -> LwzMtlr
        let shadow = build_from_words(0, &[lwz_raw(0, 1, 4), mtlr_raw(0)]);
        assert_eq!(
            shadow.get(0),
            Some(PpuInstruction::LwzMtlr {
                rt: 0,
                ra_load: 1,
                offset: 4,
            })
        );
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
    }

    #[test]
    fn super_pair_lwz_mtlr_different_reg_no_fuse() {
        let shadow = build_from_words(0, &[lwz_raw(0, 1, 4), mtlr_raw(3)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Lwz { .. })));
        assert!(matches!(shadow.get(4), Some(PpuInstruction::Mtlr { .. })));
    }

    #[test]
    fn super_pair_mflr_std() {
        // mflr r0 + std r0, 16(r1) -> MflrStd
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
        // ld r0, 16(r1) + mtlr r0 -> LdMtlr
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
        // std r3, 0(r1) + std r4, 16(r1): off2 != off1+8 -> no fuse.
        let shadow = build_from_words(0, &[std_raw(3, 1, 0), std_raw(4, 1, 16)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Std { .. })));
        assert!(matches!(shadow.get(4), Some(PpuInstruction::Std { .. })));
    }

    #[test]
    fn super_pair_std_std_different_base_no_fuse() {
        // std r3, 0(r1) + std r4, 8(r2): different RA -> no fuse.
        let shadow = build_from_words(0, &[std_raw(3, 1, 0), std_raw(4, 2, 8)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Std { .. })));
        assert!(matches!(shadow.get(4), Some(PpuInstruction::Std { .. })));
    }

    #[test]
    fn super_pair_consumed_not_chained() {
        // Three lis: only the first pair fuses; the resulting Consumed
        // must not chain with the third instruction.
        let shadow = build_from_words(0, &[li_raw(3, 1), stw_raw(3, 1, 0), li_raw(4, 2)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::LiStw { .. })));
        assert_eq!(shadow.get(4), Some(PpuInstruction::Consumed));
        assert_eq!(shadow.get(8), Some(PpuInstruction::Li { rt: 4, imm: 2 }));
    }

    #[test]
    fn super_pair_branch_blocks_fusion() {
        // li r3, 1 ; b +N ; lwz r3, 0(r1) ; cmpwi cr0, r3, 0 -- the
        // pair after the branch should still fuse; the branch ends
        // the previous block and isn't itself a fusion source.
        let shadow = build_from_words(
            0,
            &[li_raw(3, 1), b_raw(8), lwz_raw(3, 1, 0), cmpwi_raw(0, 3, 0)],
        );
        // Verify the post-branch pair fuses (regression: the branch
        // must not block fusion of subsequent pairs).
        assert!(matches!(
            shadow.get(8),
            Some(PpuInstruction::LwzCmpwi { .. })
        ));
        assert_eq!(shadow.get(12), Some(PpuInstruction::Consumed));
    }

    #[test]
    fn super_pair_stale_slot_blocks_fusion() {
        // li r3, 1 + stw r3, 0(r1) would fuse, but if either slot
        // is stale at fusion time, the pair is skipped.
        let mut shadow = build_from_words(0, &[li_raw(3, 1), stw_raw(3, 1, 0)]);
        // After build, the pair has already fused. Invalidate both
        // slots and verify they're not in fused state.
        shadow.invalidate_range(0, 8);
        assert!(shadow.get(0).is_none());
        assert!(shadow.get(4).is_none());
    }

    #[test]
    fn super_pair_multiple_pairs() {
        // Two adjacent fusable pairs.
        let shadow = build_from_words(
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
    }

    #[test]
    fn super_pair_single_instruction_shadow() {
        // Single-slot shadow: super_pair is a no-op.
        let shadow = build_from_words(0, &[li_raw(3, 1)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Li { .. })));
    }

    #[test]
    fn super_pair_empty_shadow() {
        // Empty shadow: super_pair is a no-op.
        let shadow = build_from_words(0, &[]);
        assert!(shadow.is_empty());
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

    #[test]
    fn super_pair_cmpwi_bc_aa_no_fuse() {
        // bca (AA=1) is an absolute-address branch; the fused
        // CmpwiBc::target_offset is PC-relative, so the AA=1 form must
        // remain unfused. The match guard requires aa: false.
        let bc_aa = bc_raw(0x0C, 2, 8) | 2; // AA bit
        let shadow = build_from_words(0, &[cmpwi_raw(0, 3, 42), bc_aa]);
        assert!(
            matches!(shadow.get(0), Some(PpuInstruction::Cmpwi { .. })),
            "bca must not fuse into CmpwiBc; target_offset is PC-relative"
        );
        assert!(matches!(
            shadow.get(4),
            Some(PpuInstruction::Bc { aa: true, .. })
        ));
    }

    #[test]
    fn super_pair_lwz_cmpw_zero_different_reg_no_fuse() {
        // lwz r3 + (cmpwi cr0, r4, 0 -> CmpwZero {ra: 4}) must not
        // fuse: the Lwz+CmpwZero arm requires rt == cmp_ra. Mirrors the
        // existing super_pair_lwz_cmpwi_different_reg_no_fuse coverage
        // for the quickened-zero path.
        let shadow = build_from_words(0, &[lwz_raw(3, 1, 8), cmpwi_raw(0, 4, 0)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Lwz { .. })));
        assert!(matches!(
            shadow.get(4),
            Some(PpuInstruction::CmpwZero { ra: 4, .. })
        ));
    }

    #[test]
    fn super_pair_mflr_std_different_reg_no_fuse() {
        let shadow = build_from_words(0, &[mflr_raw(0), std_raw(3, 1, 16)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Mflr { .. })));
        assert!(matches!(shadow.get(4), Some(PpuInstruction::Std { .. })));
    }

    #[test]
    fn super_pair_ld_mtlr_different_reg_no_fuse() {
        let shadow = build_from_words(0, &[ld_raw(0, 1, 16), mtlr_raw(3)]);
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Ld { .. })));
        assert!(matches!(shadow.get(4), Some(PpuInstruction::Mtlr { .. })));
    }

    #[test]
    fn super_pair_std_std_offset_overflow_no_fuse() {
        // off1 = 32760 (max DS-aligned positive offset that overflows
        // on +8). Pre-fix wrapping_add(8) wrapped to -32768 and matched
        // a distant store at offset -32768; checked_add returns None
        // and the pair stays unfused.
        let shadow = build_from_words(0, &[std_raw(3, 1, 32760), std_raw(4, 1, -32768)]);
        assert!(
            matches!(shadow.get(0), Some(PpuInstruction::Std { .. })),
            "off1 + 8 overflowing i16 must not be reported as adjacent"
        );
        assert!(matches!(shadow.get(4), Some(PpuInstruction::Std { .. })));
    }

    #[test]
    fn super_pair_block_len_with_mid_sequence_cmpwi_bc() {
        // Five-instruction sequence: three li, then cmpwi + bc. After
        // super-pairing, slots 3..4 collapse to CmpwiBc + Consumed.
        // block_len must terminate at the CmpwiBc and continue past
        // the Consumed for slots after the branch. compute_block_lengths
        // is unconditionally re-run after super-pairing in build, so a
        // CmpwiBc anywhere in the shadow is recognized as a terminator.
        let shadow = build_from_words(
            0,
            &[
                li_raw(3, 1),
                li_raw(4, 2),
                li_raw(5, 3),
                cmpwi_raw(0, 3, 5),
                bc_raw(0x0C, 2, 8),
            ],
        );
        assert!(matches!(shadow.get(0), Some(PpuInstruction::Li { .. })));
        assert!(matches!(shadow.get(4), Some(PpuInstruction::Li { .. })));
        assert!(matches!(shadow.get(8), Some(PpuInstruction::Li { .. })));
        assert!(matches!(
            shadow.get(12),
            Some(PpuInstruction::CmpwiBc { .. })
        ));
        assert_eq!(shadow.get(16), Some(PpuInstruction::Consumed));
        // Block runs slot 0 through the CmpwiBc terminator at slot 3.
        assert_eq!(shadow.block_len_at(0), 4);
        assert_eq!(shadow.block_len_at(4), 3);
        assert_eq!(shadow.block_len_at(8), 2);
        assert_eq!(shadow.block_len_at(12), 1, "CmpwiBc terminates the block");
    }
}
