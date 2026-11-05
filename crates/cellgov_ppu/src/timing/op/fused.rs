//! The issue model's view of a fused pair: two ops, in program order.

use super::fixed::{add_form, branch, branches, cmp, fused_branches, logical};
use super::model::{gpr, unit, Op, Target, LR};
use super::storage::{ea_d, fx, int_load, store, update, INT_STORE};
use crate::instruction::FusedPair;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BO_ALWAYS;
use cellgov_ps3_abi::hw::ppu_issue as k;

/// The two ops of `pair` at the state it executes from, in program
/// order.
pub(super) fn classify_fused(pair: FusedPair, state: &PpuState) -> ([Op; 2], usize) {
    let two = |a: Op, b: Op| ([a, b], 2);
    let g = |r: u8| state.gpr[r as usize];
    match pair {
        FusedPair::LwzCmpwi {
            rt,
            ra_load,
            offset,
            bf,
            ..
        } => two(
            int_load(ea_d(state, ra_load, offset), 4, rt).base(ra_load),
            cmp(bf, rt, None),
        ),
        FusedPair::LiStw {
            rt,
            imm,
            ra_store,
            store_offset,
        } => {
            let base = if ra_store == rt {
                imm as i64 as u64
            } else {
                g(ra_store)
            };
            let ea = if ra_store == 0 { 0 } else { base }.wrapping_add(store_offset as i64 as u64);
            two(
                fx(gpr(rt)),
                store(INT_STORE, ea, 4).src(gpr(rt)).base(ra_store),
            )
        }
        FusedPair::MflrStw {
            rt,
            ra_store,
            store_offset,
        }
        | FusedPair::MflrStd {
            rt,
            ra_store,
            store_offset,
        } => {
            let len = if matches!(pair, FusedPair::MflrStd { .. }) {
                8
            } else {
                4
            };
            let base = if ra_store == rt {
                state.lr()
            } else {
                g(ra_store)
            };
            let ea = if ra_store == 0 { 0 } else { base }.wrapping_add(store_offset as i64 as u64);
            two(
                Op::on(unit::BRU)
                    .src(LR)
                    .dst(gpr(rt), k::PPU_BRANCH_UNIT_LATENCY_CYCLES),
                store(INT_STORE, ea, len).src(gpr(rt)).base(ra_store),
            )
        }
        FusedPair::LwzMtlr {
            rt,
            ra_load,
            offset,
        }
        | FusedPair::LdMtlr {
            rt,
            ra_load,
            offset,
        } => {
            let len = if matches!(pair, FusedPair::LdMtlr { .. }) {
                8
            } else {
                4
            };
            two(
                int_load(ea_d(state, ra_load, offset), len, rt).base(ra_load),
                Op::on(unit::BRU)
                    .src(gpr(rt))
                    .dst(LR, k::PPU_BRANCH_UNIT_LATENCY_CYCLES),
            )
        }
        FusedPair::StdStd {
            rs1,
            rs2,
            ra,
            offset1,
        } => {
            let ea = ea_d(state, ra, offset1);
            two(
                store(INT_STORE, ea, 8).src(gpr(rs1)).base(ra),
                store(INT_STORE, ea.wrapping_add(8), 8)
                    .src(gpr(rs2))
                    .base(ra),
            )
        }
        FusedPair::LdLd {
            rt1,
            rt2,
            ra,
            offset1,
        } => {
            let ea = ea_d(state, ra, offset1);
            two(
                int_load(ea, 8, rt1).base(ra),
                int_load(ea.wrapping_add(8), 8, rt2).base(ra),
            )
        }
        FusedPair::LbzuCmpwi {
            rt,
            ra,
            offset,
            bf,
            cmp_ra,
        } => two(
            update(int_load(ea_d(state, ra, offset), 1, rt).base(ra), ra),
            cmp(bf, cmp_ra, None),
        ),
        FusedPair::LduLdu {
            rt1,
            ra1,
            off1,
            rt2,
            ra2,
            off2,
        } => {
            let ea1 = ea_d(state, ra1, off1);
            let base2 = if ra2 == ra1 { ea1 } else { g(ra2) };
            two(
                update(int_load(ea1, 8, rt1).base(ra1), ra1),
                update(
                    int_load(base2.wrapping_add(off2 as i64 as u64), 8, rt2).base(ra2),
                    ra2,
                ),
            )
        }
        FusedPair::CmpdNor {
            bf,
            ra,
            rb,
            nor_ra,
            nor_rs,
            nor_rb,
        } => two(
            cmp(bf, ra, Some(rb)),
            logical(
                Op::on(unit::FXU).src(gpr(nor_rs)).src(gpr(nor_rb)),
                nor_ra,
                false,
            ),
        ),
        FusedPair::ExtswBlr { ra, rs, bo, bi } => two(
            logical(Op::on(unit::FXU).src(gpr(rs)), ra, false),
            branch(bo, bi, false, Target::Link, branches(state, bo, bi)),
        ),
        FusedPair::LisOri { rt, .. } => two(fx(gpr(rt)), fx(gpr(rt)).src(gpr(rt))),
        FusedPair::OrAdd {
            ra,
            rs,
            rb,
            rt,
            add_ra,
            add_rb,
        } => two(
            logical(Op::on(unit::FXU).src(gpr(rs)).src(gpr(rb)), ra, false),
            add_form(rt, &[add_ra, add_rb], false, false),
        ),
        FusedPair::CmpwiBc {
            bf,
            ra,
            imm,
            bo,
            bi,
            ..
        } => {
            let operands = (g(ra) as i32, i32::from(imm));
            let taken = fused_branches(state, bf, operands, bo, bi);
            two(
                cmp(bf, ra, None),
                branch(bo, bi, false, Target::Direct, taken),
            )
        }
        FusedPair::CmpwBc {
            bf, ra, rb, bo, bi, ..
        } => {
            let operands = (g(ra) as i32, g(rb) as i32);
            let taken = fused_branches(state, bf, operands, bo, bi);
            two(
                cmp(bf, ra, Some(rb)),
                branch(bo, bi, false, Target::Direct, taken),
            )
        }
        FusedPair::CmpdiBc {
            bf,
            ra,
            imm,
            bo,
            bi,
            ..
        } => {
            let operands = (g(ra) as i64, i64::from(imm));
            let taken = fused_branches(state, bf, operands, bo, bi);
            two(
                cmp(bf, ra, None),
                branch(bo, bi, false, Target::Direct, taken),
            )
        }
        FusedPair::CmpdBc {
            bf, ra, rb, bo, bi, ..
        } => {
            let operands = (g(ra) as i64, g(rb) as i64);
            let taken = fused_branches(state, bf, operands, bo, bi);
            two(
                cmp(bf, ra, Some(rb)),
                branch(bo, bi, false, Target::Direct, taken),
            )
        }
        FusedPair::CmplwBc {
            bf, ra, rb, bo, bi, ..
        } => {
            let operands = (g(ra) as u32, g(rb) as u32);
            let taken = fused_branches(state, bf, operands, bo, bi);
            two(
                cmp(bf, ra, Some(rb)),
                branch(bo, bi, false, Target::Direct, taken),
            )
        }
        FusedPair::OrRlwinm {
            ra,
            rs,
            rb,
            rlwinm_ra,
            rlwinm_rs,
            rc,
            ..
        } => two(
            logical(Op::on(unit::FXU).src(gpr(rs)).src(gpr(rb)), ra, false),
            logical(Op::on(unit::FXU).src(gpr(rlwinm_rs)), rlwinm_ra, rc),
        ),
        FusedPair::LwzLwz {
            rt1,
            ra1,
            off1,
            rt2,
            ra2,
            off2,
        } => two(
            int_load(ea_d(state, ra1, off1), 4, rt1).base(ra1),
            int_load(ea_d(state, ra2, off2), 4, rt2).base(ra2),
        ),
        FusedPair::LdRlwinm {
            rt,
            offset,
            ra,
            rs,
            rc,
            ..
        } => two(
            int_load(ea_d(state, 1, offset), 8, rt).base(1),
            logical(Op::on(unit::FXU).src(gpr(rs)), ra, rc),
        ),
        FusedPair::AddiBc { rt, ra, bo, bi, .. } => {
            let add = fx(gpr(rt));
            two(
                if ra == 0 { add } else { add.src(gpr(ra)) },
                branch(bo, bi, false, Target::Direct, branches(state, bo, bi)),
            )
        }
        FusedPair::StdLwz {
            rs,
            store_offset,
            rt,
            ra,
            offset,
        } => two(
            store(INT_STORE, ea_d(state, 1, store_offset), 8)
                .src(gpr(rs))
                .base(1),
            int_load(ea_d(state, ra, offset), 4, rt).base(ra),
        ),
        FusedPair::LdLdAny {
            rt1,
            ra1,
            off1,
            rt2,
            ra2,
            off2,
        } => two(
            int_load(ea_d(state, ra1, off1), 8, rt1).base(ra1),
            int_load(ea_d(state, ra2, off2), 8, rt2).base(ra2),
        ),
        FusedPair::OrB { ra, rs, rb, rc, .. } => two(
            logical(Op::on(unit::FXU).src(gpr(rs)).src(gpr(rb)), ra, rc),
            branch(PPC_BO_ALWAYS, 0, false, Target::Direct, true),
        ),
        FusedPair::LbzLbz {
            rt1,
            ra1,
            off1,
            rt2,
            ra2,
            off2,
        } => two(
            int_load(ea_d(state, ra1, off1), 1, rt1).base(ra1),
            int_load(ea_d(state, ra2, off2), 1, rt2).base(ra2),
        ),
        FusedPair::MtlrAddi { rs, rt, ra, .. } => {
            let add = fx(gpr(rt));
            two(
                Op::on(unit::BRU)
                    .src(gpr(rs))
                    .dst(LR, k::PPU_BRANCH_UNIT_LATENCY_CYCLES),
                if ra == 0 { add } else { add.src(gpr(ra)) },
            )
        }
        FusedPair::RldiclRldicl {
            ra1,
            rs1,
            ra2,
            rs2,
            rc,
            ..
        } => two(
            logical(Op::on(unit::FXU).src(gpr(rs1)), ra1, rc & 1 != 0),
            logical(Op::on(unit::FXU).src(gpr(rs2)), ra2, rc & 2 != 0),
        ),
        FusedPair::OriAnd {
            ra,
            rs,
            imm,
            and_ra,
            and_rs,
            and_rb,
            rc,
        } => two(
            // `ori 0,0,0` times as the quickened `Nop` does.
            if ra == 0 && rs == 0 && imm == 0 {
                Op::on(unit::FXU)
            } else {
                fx(gpr(ra)).src(gpr(rs))
            },
            logical(
                Op::on(unit::FXU).src(gpr(and_rs)).src(gpr(and_rb)),
                and_ra,
                rc,
            ),
        ),
        FusedPair::RldicrOris {
            ra,
            rs,
            rc,
            oris_ra,
            oris_rs,
            ..
        } => two(
            logical(Op::on(unit::FXU).src(gpr(rs)), ra, rc),
            fx(gpr(oris_ra)).src(gpr(oris_rs)),
        ),
        FusedPair::OrOr {
            ra1,
            rs1,
            rb1,
            rc1,
            ra2,
            rs2,
            rb2,
            rc2,
        } => two(
            logical(Op::on(unit::FXU).src(gpr(rs1)).src(gpr(rb1)), ra1, rc1),
            logical(Op::on(unit::FXU).src(gpr(rs2)).src(gpr(rb2)), ra2, rc2),
        ),
        FusedPair::OrBc {
            ra,
            rs,
            rb,
            rc,
            bo,
            bi,
            ..
        } => {
            // [PPC-Book1 p:67 s:3.3.11] a recording or sets CR0 from its
            // 64-bit result, and the bc reads that CR0.
            let taken = if rc {
                fused_branches(state, 0, ((g(rs) | g(rb)) as i64, 0), bo, bi)
            } else {
                branches(state, bo, bi)
            };
            two(
                logical(Op::on(unit::FXU).src(gpr(rs)).src(gpr(rb)), ra, rc),
                branch(bo, bi, false, Target::Direct, taken),
            )
        }
        FusedPair::OrLwz {
            ra,
            rs,
            rb,
            rc,
            rt,
            lwz_ra,
            offset,
        } => {
            let base = match lwz_ra {
                0 => 0,
                r if r == ra => g(rs) | g(rb),
                r => g(r),
            };
            let ea = base.wrapping_add(offset as i64 as u64);
            two(
                logical(Op::on(unit::FXU).src(gpr(rs)).src(gpr(rb)), ra, rc),
                int_load(ea, 4, rt).base(lwz_ra),
            )
        }
        FusedPair::OrBctrl {
            ra,
            rs,
            rb,
            rc,
            bo,
            bi,
        } => two(
            logical(Op::on(unit::FXU).src(gpr(rs)).src(gpr(rb)), ra, rc),
            branch(bo, bi, true, Target::Count, branches(state, bo, bi)),
        ),
        FusedPair::LdAddi {
            rt,
            offset,
            addi_rt,
            addi_ra,
            ..
        } => {
            let add = fx(gpr(addi_rt));
            two(
                int_load(ea_d(state, 1, offset), 8, rt).base(1),
                if addi_ra == 0 {
                    add
                } else {
                    add.src(gpr(addi_ra))
                },
            )
        }
        FusedPair::LwzOr {
            rt,
            ra,
            offset,
            or_ra,
            or_rs,
            or_rb,
            rc,
        } => two(
            int_load(ea_d(state, ra, offset), 4, rt).base(ra),
            logical(Op::on(unit::FXU).src(gpr(or_rs)).src(gpr(or_rb)), or_ra, rc),
        ),
    }
}
