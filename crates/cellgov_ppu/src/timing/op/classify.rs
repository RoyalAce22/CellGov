//! The issue model's view of one instruction.

use super::fixed::{add_form, branch, branches, cmp, cr_logical, divide, logical, multiply};
use super::float::{fp59, fp63, fp_load};
use super::fused::classify_fused;
use super::model::{crf, fpr, gpr, unit, vr, Kind, LoadClass, Op, Target, CTR, LR, XER};
use super::storage::{
    ea_d, ea_x, fx, int_load, load, load_algebraic, multiple, store, update, INT_STORE, VSU_MEM,
};
use super::vector::{vector, vx_class};
use crate::instruction::ops::VaOp;
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BO_ALWAYS;
use cellgov_ps3_abi::hw::ppu_issue as k;

/// The model's view of `insn` at the state it executes from: one op, or
/// two for a fused pair, in program order.
pub(crate) fn classify(insn: &I, state: &PpuState) -> ([Op; 2], usize) {
    let none = Op::on(0);
    let one = |op: Op| ([op, none], 1);
    match *insn {
        I::Lwz { rt, ra, imm } => one(int_load(ea_d(state, ra, imm), 4, rt).base(ra)),
        I::Lbz { rt, ra, imm } => one(int_load(ea_d(state, ra, imm), 1, rt).base(ra)),
        I::Lhz { rt, ra, imm } => one(int_load(ea_d(state, ra, imm), 2, rt).base(ra)),
        I::Ld { rt, ra, imm } => one(int_load(ea_d(state, ra, imm), 8, rt).base(ra)),
        I::Lwzu { rt, ra, imm } => one(update(int_load(ea_d(state, ra, imm), 4, rt).base(ra), ra)),
        I::Lbzu { rt, ra, imm } => one(update(int_load(ea_d(state, ra, imm), 1, rt).base(ra), ra)),
        I::Lhzu { rt, ra, imm } => one(update(int_load(ea_d(state, ra, imm), 2, rt).base(ra), ra)),
        I::Ldu { rt, ra, imm } => one(update(int_load(ea_d(state, ra, imm), 8, rt).base(ra), ra)),
        I::Lha { rt, ra, imm } => {
            one(load_algebraic(ea_d(state, ra, imm), 2, rt, k::PPU_LHA_MICROWORDS).base(ra))
        }
        I::Lhau { rt, ra, imm } => one(update(
            load_algebraic(ea_d(state, ra, imm), 2, rt, 8).base(ra),
            ra,
        )),
        I::Lwa { rt, ra, imm } => one(load_algebraic(ea_d(state, ra, imm), 4, rt, 13).base(ra)),
        I::Lmw { rt, ra, imm } => {
            one(multiple(ea_d(state, ra, imm), rt, 4 * (32 - rt as u64), false, false).base(ra))
        }
        I::Stmw { rs, ra, imm } => {
            one(multiple(ea_d(state, ra, imm), rs, 4 * (32 - rs as u64), true, false).base(ra))
        }
        I::Stw { rs, ra, imm } => one(store(INT_STORE, ea_d(state, ra, imm), 4)
            .src(gpr(rs))
            .base(ra)),
        I::Stb { rs, ra, imm } => one(store(INT_STORE, ea_d(state, ra, imm), 1)
            .src(gpr(rs))
            .base(ra)),
        I::Sth { rs, ra, imm } => one(store(INT_STORE, ea_d(state, ra, imm), 2)
            .src(gpr(rs))
            .base(ra)),
        I::Std { rs, ra, imm } => one(store(INT_STORE, ea_d(state, ra, imm), 8)
            .src(gpr(rs))
            .base(ra)),
        I::Stwu { rs, ra, imm } => one(update(
            store(INT_STORE, ea_d(state, ra, imm), 4)
                .src(gpr(rs))
                .base(ra),
            ra,
        )),
        I::Stbu { rs, ra, imm } => one(update(
            store(INT_STORE, ea_d(state, ra, imm), 1)
                .src(gpr(rs))
                .base(ra),
            ra,
        )),
        I::Sthu { rs, ra, imm } => one(update(
            store(INT_STORE, ea_d(state, ra, imm), 2)
                .src(gpr(rs))
                .base(ra),
            ra,
        )),
        I::Stdu { rs, ra, imm } => one(update(
            store(INT_STORE, ea_d(state, ra, imm), 8)
                .src(gpr(rs))
                .base(ra),
            ra,
        )),
        I::Addi { rt, ra, .. } | I::Addis { rt, ra, .. } => {
            let op = fx(gpr(rt));
            one(if ra == 0 { op } else { op.src(gpr(ra)) })
        }
        I::Subfic { rt, ra, .. } | I::Addic { rt, ra, .. } => {
            one(fx(gpr(rt)).src(gpr(ra)).dst(XER, k::PPU_FX_LATENCY_CYCLES))
        }
        I::AddicDot { rt, ra, .. } => one(fx(gpr(rt))
            .src(gpr(ra))
            .dst(XER, k::PPU_FX_LATENCY_CYCLES)
            .record_cr0(true)),
        I::Mulli { rt, ra, .. } => one(multiply(
            ra,
            None,
            rt,
            k::PPU_MULLI_LATENCY_CYCLES,
            false,
            false,
        )),
        I::Add { rt, ra, rb, oe, rc } | I::Subf { rt, ra, rb, oe, rc } => {
            one(add_form(rt, &[ra, rb], oe, rc))
        }
        I::Subfc {
            rt,
            ra,
            rb,
            oe: _,
            rc,
        }
        | I::Addc {
            rt,
            ra,
            rb,
            oe: _,
            rc,
        } => one(add_form(rt, &[ra, rb], true, rc)),
        I::Subfe {
            rt,
            ra,
            rb,
            oe: _,
            rc,
        }
        | I::Adde {
            rt,
            ra,
            rb,
            oe: _,
            rc,
        } => one(add_form(rt, &[ra, rb], true, rc).src(XER)),
        I::Neg { rt, ra, oe, rc } => one(add_form(rt, &[ra], oe, rc)),
        I::Addze { rt, ra, oe: _, rc }
        | I::Subfze { rt, ra, oe: _, rc }
        | I::Subfme { rt, ra, oe: _, rc }
        | I::Addme { rt, ra, oe: _, rc } => one(add_form(rt, &[ra], true, rc).src(XER)),
        I::Mullw { rt, ra, rb, oe, rc } => one(multiply(
            ra,
            Some(rb),
            rt,
            k::PPU_MULLW_LATENCY_CYCLES,
            oe,
            rc,
        )),
        I::Mulhw { rt, ra, rb, rc } | I::Mulhwu { rt, ra, rb, rc } => one(multiply(
            ra,
            Some(rb),
            rt,
            k::PPU_MULLW_LATENCY_CYCLES,
            false,
            rc,
        )),
        I::Mulld { rt, ra, rb, oe, rc } => one(multiply(
            ra,
            Some(rb),
            rt,
            k::PPU_MULLD_LATENCY_CYCLES,
            oe,
            rc,
        )),
        I::Mulhd { rt, ra, rb, rc } | I::Mulhdu { rt, ra, rb, rc } => one(multiply(
            ra,
            Some(rb),
            rt,
            k::PPU_MULLD_LATENCY_CYCLES,
            false,
            rc,
        )),
        I::Ldarx { rt, ra, rb } => one(load(unit::LSU, ea_x(state, ra, rb), 8, LoadClass::Reserve)
            .dst(gpr(rt), k::PPU_LOAD_FX_USE_CYCLES)
            .base(ra)
            .index(rb)),
        I::Lwarx { rt, ra, rb } => one(load(unit::LSU, ea_x(state, ra, rb), 4, LoadClass::Reserve)
            .dst(gpr(rt), k::PPU_LOAD_FX_USE_CYCLES)
            .base(ra)
            .index(rb)),
        I::Stdcx { rs, ra, rb } | I::Stwcx { rs, ra, rb } => {
            let len = if matches!(insn, I::Stdcx { .. }) {
                8
            } else {
                4
            };
            one(Op::on(INT_STORE)
                .kind(Kind::Store {
                    ea: ea_x(state, ra, rb),
                    len,
                    conditional: true,
                })
                .src(gpr(rs))
                .base(ra)
                .index(rb)
                .dst(crf(0), k::PPU_L2_LOAD_USE_CYCLES))
        }
        // `ori 0,0,0` is the preferred no-op, which the shadow quickens to
        // `Nop`; both time the same.
        I::Ori {
            ra: 0,
            rs: 0,
            imm: 0,
        } => one(Op::on(unit::FXU)),
        I::Xori { ra, rs, .. }
        | I::Xoris { ra, rs, .. }
        | I::Ori { ra, rs, .. }
        | I::Oris { ra, rs, .. } => one(fx(gpr(ra)).src(gpr(rs))),
        I::Divw { rt, ra, rb, oe, rc } => one(divide(state, rt, ra, rb, oe, rc, (true, true))),
        I::Divwu { rt, ra, rb, oe, rc } => one(divide(state, rt, ra, rb, oe, rc, (true, false))),
        I::Divd { rt, ra, rb, oe, rc } => one(divide(state, rt, ra, rb, oe, rc, (false, true))),
        I::Divdu { rt, ra, rb, oe, rc } => one(divide(state, rt, ra, rb, oe, rc, (false, false))),
        I::And { ra, rs, rb, rc }
        | I::Andc { ra, rs, rb, rc }
        | I::Nor { ra, rs, rb, rc }
        | I::Xor { ra, rs, rb, rc }
        | I::Eqv { ra, rs, rb, rc }
        | I::Nand { ra, rs, rb, rc }
        | I::Orc { ra, rs, rb, rc }
        | I::Or { ra, rs, rb, rc } => {
            one(logical(Op::on(unit::FXU).src(gpr(rs)).src(gpr(rb)), ra, rc))
        }
        I::AndiDot { ra, rs, .. } | I::AndisDot { ra, rs, .. } => {
            one(logical(Op::on(unit::FXU).src(gpr(rs)), ra, true))
        }
        I::Slw { ra, rs, rb, rc }
        | I::Srw { ra, rs, rb, rc }
        | I::Sld { ra, rs, rb, rc }
        | I::Srd { ra, rs, rb, rc } => one(Op::on(unit::FXU)
            .src(gpr(rs))
            .src(gpr(rb))
            .indirect_shift(gpr(ra), rc)),
        I::Sraw { ra, rs, rb, rc } | I::Srad { ra, rs, rb, rc } => one(Op::on(unit::FXU)
            .src(gpr(rs))
            .src(gpr(rb))
            .dst(XER, 3 + k::PPU_FX_LATENCY_CYCLES)
            .indirect_shift(gpr(ra), rc)),
        I::Srawi { ra, rs, rc, .. } | I::Sradi { ra, rs, rc, .. } => one(logical(
            Op::on(unit::FXU)
                .src(gpr(rs))
                .dst(XER, k::PPU_FX_LATENCY_CYCLES),
            ra,
            rc,
        )),
        I::Cntlzw { ra, rs, rc }
        | I::Cntlzd { ra, rs, rc }
        | I::Extsh { ra, rs, rc }
        | I::Extsb { ra, rs, rc }
        | I::Extsw { ra, rs, rc } => one(logical(Op::on(unit::FXU).src(gpr(rs)), ra, rc)),
        // The PPE raises an illegal- or privileged-instruction interrupt for these. The
        // instruction never retires, so it reads and writes nothing.
        I::Popcntb { .. } | I::Mcrxr { .. } | I::PpeIllegal { .. } | I::PpePrivileged { .. } => {
            one(Op::on(unit::FXU))
        }
        I::Tw { ra, rb, .. } | I::Td { ra, rb, .. } => {
            one(Op::on(unit::FXU).src(gpr(ra)).src(gpr(rb)))
        }
        I::Twi { ra, .. } | I::Tdi { ra, .. } => one(Op::on(unit::FXU).src(gpr(ra))),
        I::Cmpwi { bf, ra, .. }
        | I::Cmplwi { bf, ra, .. }
        | I::Cmpdi { bf, ra, .. }
        | I::Cmpldi { bf, ra, .. } => one(cmp(bf, ra, None)),
        I::CmpwZero { bf, ra } | I::CmpdZero { bf, ra } => one(cmp(bf, ra, None)),
        I::Cmpw { bf, ra, rb }
        | I::Cmplw { bf, ra, rb }
        | I::Cmpd { bf, ra, rb }
        | I::Cmpld { bf, ra, rb } => one(cmp(bf, ra, Some(rb))),
        I::B { link, .. } => one(branch(PPC_BO_ALWAYS, 0, link, Target::Direct, true)),
        I::Bc { bo, bi, link, .. } => one(branch(
            bo,
            bi,
            link,
            Target::Direct,
            branches(state, bo, bi),
        )),
        I::Bdnz { bo, bi, .. } | I::BcTrue { bo, bi, .. } | I::BcFalse { bo, bi, .. } => one(
            branch(bo, bi, false, Target::Direct, branches(state, bo, bi)),
        ),
        I::Bclr { bo, bi, link } => {
            one(branch(bo, bi, link, Target::Link, branches(state, bo, bi)))
        }
        I::Blr { bo, bi } => one(branch(bo, bi, false, Target::Link, branches(state, bo, bi))),
        I::Bcctr { bo, bi, link } => {
            one(branch(bo, bi, link, Target::Count, branches(state, bo, bi)))
        }
        I::Mcrf { crfd, crfs } => one(Op::on(unit::BRU)
            .src(crf(crfs))
            .dst(crf(crfd), k::PPU_BRANCH_UNIT_LATENCY_CYCLES)),
        I::Crand { bt, ba, bb }
        | I::Crandc { bt, ba, bb }
        | I::Cror { bt, ba, bb }
        | I::Crorc { bt, ba, bb }
        | I::Crxor { bt, ba, bb }
        | I::Crnand { bt, ba, bb }
        | I::Crnor { bt, ba, bb }
        | I::Creqv { bt, ba, bb } => one(cr_logical(bt, ba, bb)),
        I::Lwzx { rt, ra, rb } | I::Lwbrx { rt, ra, rb } => {
            one(int_load(ea_x(state, ra, rb), 4, rt).base(ra).index(rb))
        }
        I::Lbzx { rt, ra, rb } => one(int_load(ea_x(state, ra, rb), 1, rt).base(ra).index(rb)),
        I::Lhzx { rt, ra, rb } | I::Lhbrx { rt, ra, rb } => {
            one(int_load(ea_x(state, ra, rb), 2, rt).base(ra).index(rb))
        }
        I::Ldx { rt, ra, rb } | I::Ldbrx { rt, ra, rb } => {
            one(int_load(ea_x(state, ra, rb), 8, rt).base(ra).index(rb))
        }
        I::Lwzux { rt, ra, rb } => one(update(
            int_load(ea_x(state, ra, rb), 4, rt).base(ra).index(rb),
            ra,
        )),
        I::Lbzux { rt, ra, rb } => one(update(
            int_load(ea_x(state, ra, rb), 1, rt).base(ra).index(rb),
            ra,
        )),
        I::Lhzux { rt, ra, rb } => one(update(
            int_load(ea_x(state, ra, rb), 2, rt).base(ra).index(rb),
            ra,
        )),
        I::Ldux { rt, ra, rb } => one(update(
            int_load(ea_x(state, ra, rb), 8, rt).base(ra).index(rb),
            ra,
        )),
        I::Lhax { rt, ra, rb } => one(load_algebraic(ea_x(state, ra, rb), 2, rt, 8)
            .base(ra)
            .index(rb)),
        I::Lhaux { rt, ra, rb } => one(update(
            load_algebraic(ea_x(state, ra, rb), 2, rt, 8)
                .base(ra)
                .index(rb),
            ra,
        )),
        I::Lwax { rt, ra, rb } => one(load_algebraic(ea_x(state, ra, rb), 4, rt, 12)
            .base(ra)
            .index(rb)),
        I::Lwaux { rt, ra, rb } => one(update(
            load_algebraic(ea_x(state, ra, rb), 4, rt, 12)
                .base(ra)
                .index(rb),
            ra,
        )),
        I::Stwx { rs, ra, rb } | I::Stwbrx { rs, ra, rb } => {
            one(store(INT_STORE, ea_x(state, ra, rb), 4)
                .src(gpr(rs))
                .base(ra)
                .index(rb))
        }
        I::Stdx { rs, ra, rb } | I::Sdbrx { rs, ra, rb } => {
            one(store(INT_STORE, ea_x(state, ra, rb), 8)
                .src(gpr(rs))
                .base(ra)
                .index(rb))
        }
        I::Stbx { rs, ra, rb } => one(store(INT_STORE, ea_x(state, ra, rb), 1)
            .src(gpr(rs))
            .base(ra)
            .index(rb)),
        I::Sthx { rs, ra, rb } | I::Sthbrx { rs, ra, rb } => {
            one(store(INT_STORE, ea_x(state, ra, rb), 2)
                .src(gpr(rs))
                .base(ra)
                .index(rb))
        }
        I::Stdux { rs, ra, rb } => one(update(
            store(INT_STORE, ea_x(state, ra, rb), 8)
                .src(gpr(rs))
                .base(ra)
                .index(rb),
            ra,
        )),
        I::Stwux { rs, ra, rb } => one(update(
            store(INT_STORE, ea_x(state, ra, rb), 4)
                .src(gpr(rs))
                .base(ra)
                .index(rb),
            ra,
        )),
        I::Sthux { rs, ra, rb } => one(update(
            store(INT_STORE, ea_x(state, ra, rb), 2)
                .src(gpr(rs))
                .base(ra)
                .index(rb),
            ra,
        )),
        I::Stbux { rs, ra, rb } => one(update(
            store(INT_STORE, ea_x(state, ra, rb), 1)
                .src(gpr(rs))
                .base(ra)
                .index(rb),
            ra,
        )),
        I::Lswi { rt, ra, nb } => {
            let bytes = if nb == 0 { 32 } else { nb as u64 };
            one(multiple(ea_d(state, ra, 0), rt, bytes, false, true).base(ra))
        }
        I::Stswi { rs, ra, nb } => {
            let bytes = if nb == 0 { 32 } else { nb as u64 };
            one(multiple(ea_d(state, ra, 0), rs, bytes, true, true).base(ra))
        }
        I::Lswx { rt, ra, rb } => {
            let bytes = state.xer() & 0x7F;
            one(multiple(ea_x(state, ra, rb), rt, bytes, false, true)
                .base(ra)
                .index(rb)
                .src(XER))
        }
        I::Stswx { rs, ra, rb } => {
            let bytes = state.xer() & 0x7F;
            one(multiple(ea_x(state, ra, rb), rs, bytes, true, true)
                .base(ra)
                .index(rb)
                .src(XER))
        }
        I::Mftb { rt } | I::Mftbu { rt } => one(Op::on(unit::FXU)
            .dst(gpr(rt), k::PPU_MFTB_LATENCY_CYCLES)
            .kind(Kind::Unpipelined(k::PPU_MFTB_ISSUE_CYCLES as u16))),
        I::Mfcr { rt } | I::Mfocrf { rt, .. } => {
            let mut op = Op::on(unit::BRU)
                .dst(gpr(rt), k::PPU_MFCR_CYCLES)
                .kind(Kind::Unpipelined(k::PPU_MFCR_CYCLES as u16));
            for f in 0..8 {
                op = op.src(crf(f));
            }
            one(op)
        }
        I::Mtcrf { rs, crm } => {
            // [CBE-Handbook p:729 s:A.1.3.1] `mtcrf` with bit 11 zero: 9
            // microwords.
            let mut op = Op::on(unit::FXU).src(gpr(rs)).microcode(9, 9);
            for f in (0..8u8).filter(|f| crm & (0x80 >> f) != 0).take(3) {
                op = op.dst(crf(f), 9);
            }
            one(op)
        }
        I::Mtocrf { rs, crm } => {
            let field = crm.leading_zeros() as u8;
            one(Op::on(unit::FXU)
                .src(gpr(rs))
                .dst(crf(field.min(7)), k::PPU_COMPARE_LATENCY_CYCLES))
        }
        I::Mflr { rt } => one(Op::on(unit::BRU)
            .src(LR)
            .dst(gpr(rt), k::PPU_BRANCH_UNIT_LATENCY_CYCLES)),
        I::Mfctr { rt } => one(Op::on(unit::BRU)
            .src(CTR)
            .dst(gpr(rt), k::PPU_BRANCH_UNIT_LATENCY_CYCLES)),
        I::Mtlr { rs } => one(Op::on(unit::BRU)
            .src(gpr(rs))
            .dst(LR, k::PPU_BRANCH_UNIT_LATENCY_CYCLES)),
        I::Mtctr { rs } => one(Op::on(unit::BRU)
            .src(gpr(rs))
            .dst(CTR, k::PPU_BRANCH_UNIT_LATENCY_CYCLES)),
        I::Mfxer { rt } => one(fx(gpr(rt)).src(XER)),
        I::Mtxer { rs } => one(Op::on(unit::FXU)
            .src(gpr(rs))
            .dst(XER, k::PPU_FX_LATENCY_CYCLES)),
        I::Mfvrsave { rt } | I::Mfctrl { rt } | I::Mfsprg3 { rt } => one(fx(gpr(rt))),
        I::Mtvrsave { rs } => one(Op::on(unit::FXU).src(gpr(rs))),
        I::Rlwinm { ra, rs, rc, .. }
        | I::Rldicl { ra, rs, rc, .. }
        | I::Rldicr { ra, rs, rc, .. }
        | I::Rldic { ra, rs, rc, .. } => one(logical(Op::on(unit::FXU).src(gpr(rs)), ra, rc)),
        I::RlwinmMask { ra, rs, .. } => one(logical(Op::on(unit::FXU).src(gpr(rs)), ra, false)),
        I::Rlwimi { ra, rs, rc, .. } | I::Rldimi { ra, rs, rc, .. } => {
            one(logical(Op::on(unit::FXU).src(gpr(rs)).src(gpr(ra)), ra, rc))
        }
        I::Rlwnm { ra, rs, rb, rc, .. }
        | I::Rldcl { ra, rs, rb, rc, .. }
        | I::Rldcr { ra, rs, rb, rc, .. } => one(Op::on(unit::FXU)
            .src(gpr(rs))
            .src(gpr(rb))
            .indirect_shift(gpr(ra), rc)),
        I::Vx { op, rc, vt, va, vb } => {
            let (units, permute, lat) = vx_class(op);
            let srcs: &[u8] = match op.shape() {
                crate::instruction::ops::VxShape::VdVaVb => &[va, vb],
                crate::instruction::ops::VxShape::VdVb
                | crate::instruction::ops::VxShape::VdVbUimm => &[vb],
                crate::instruction::ops::VxShape::VdSimm => &[],
            };
            let op = vector(units, permute, lat, vt, srcs);
            one(if rc { op.dst(crf(6), lat) } else { op })
        }
        I::Va { op, vt, va, vb, vc } => {
            let (units, permute, lat) = match op {
                VaOp::Vperm | VaOp::Vsldoi => (unit::VSU2, true, k::PPU_VMX_PERMUTE_LATENCY_CYCLES),
                VaOp::Vsel => (unit::VSU1, false, k::PPU_VMX_SIMPLE_LATENCY_CYCLES),
                VaOp::Vmaddfp | VaOp::Vnmsubfp => {
                    (unit::VSU1, false, k::PPU_VMX_FLOAT_LATENCY_CYCLES)
                }
                VaOp::Vmhaddshs
                | VaOp::Vmhraddshs
                | VaOp::Vmladduhm
                | VaOp::Vmsumubm
                | VaOp::Vmsummbm
                | VaOp::Vmsumuhm
                | VaOp::Vmsumuhs
                | VaOp::Vmsumshm
                | VaOp::Vmsumshs => (unit::VSU1, false, k::PPU_VMX_COMPLEX_LATENCY_CYCLES),
            };
            one(vector(units, permute, lat, vt, &[va, vb, vc]))
        }
        I::Vxor { vt, va, vb } => one(vector(
            unit::VSU1,
            false,
            k::PPU_VMX_SIMPLE_LATENCY_CYCLES,
            vt,
            &[va, vb],
        )),
        I::Vsldoi { vt, va, vb, .. } => one(vector(
            unit::VSU2,
            true,
            k::PPU_VMX_PERMUTE_LATENCY_CYCLES,
            vt,
            &[va, vb],
        )),
        // [CBE-Handbook p:742 s:A.3.2] `mfvscr` and `mtvscr` are simple
        // and stall at issue until every older instruction completes; the
        // younger vector-scalar instructions wait for an `mtvscr`. The
        // `ppu_issue_vscr` capture gives the cycles the table leaves out.
        I::Mfvscr { vt } => {
            one(vector(unit::VSU1, false, k::PPU_MFVSCR_LATENCY_CYCLES, vt, &[]).kind(Kind::Drain))
        }
        I::Mtvscr { vb } => one(Op::on(unit::VSU1).src(vr(vb)).kind(Kind::VscrWrite)),
        I::Lvlx { vt, ra, rb }
        | I::Lvrx { vt, ra, rb }
        | I::Lvlxl { vt, ra, rb }
        | I::Lvrxl { vt, ra, rb }
        | I::Lvebx { vt, ra, rb }
        | I::Lvehx { vt, ra, rb }
        | I::Lvewx { vt, ra, rb }
        | I::Lvx { vt, ra, rb }
        | I::Lvxl { vt, ra, rb } => {
            one(
                load(VSU_MEM, ea_x(state, ra, rb) & !15, 16, LoadClass::Vector)
                    .dst(vr(vt), k::PPU_VMX_LOAD_LATENCY_CYCLES)
                    .base(ra)
                    .index(rb),
            )
        }
        I::Lvsl { vt, ra, rb } | I::Lvsr { vt, ra, rb } => one(Op::on(VSU_MEM)
            .dst(vr(vt), k::PPU_VMX_LOAD_LATENCY_CYCLES)
            .base(ra)
            .index(rb)),
        I::Stvlx { vs, ra, rb }
        | I::Stvrx { vs, ra, rb }
        | I::Stvlxl { vs, ra, rb }
        | I::Stvrxl { vs, ra, rb }
        | I::Stvebx { vs, ra, rb }
        | I::Stvehx { vs, ra, rb }
        | I::Stvewx { vs, ra, rb }
        | I::Stvx { vs, ra, rb }
        | I::Stvxl { vs, ra, rb } => one(store(VSU_MEM, ea_x(state, ra, rb) & !15, 16)
            .src(vr(vs))
            .base(ra)
            .index(rb)),
        I::Lfs { frt, ra, imm } => one(fp_load(ea_d(state, ra, imm), 4, frt).base(ra)),
        I::Lfd { frt, ra, imm } => one(fp_load(ea_d(state, ra, imm), 8, frt).base(ra)),
        I::Lfsu { frt, ra, imm } => one(update(fp_load(ea_d(state, ra, imm), 4, frt).base(ra), ra)),
        I::Lfdu { frt, ra, imm } => one(update(fp_load(ea_d(state, ra, imm), 8, frt).base(ra), ra)),
        I::Lfsx { frt, ra, rb } => one(fp_load(ea_x(state, ra, rb), 4, frt).base(ra).index(rb)),
        I::Lfdx { frt, ra, rb } => one(fp_load(ea_x(state, ra, rb), 8, frt).base(ra).index(rb)),
        I::Lfsux { frt, ra, rb } => one(update(
            fp_load(ea_x(state, ra, rb), 4, frt).base(ra).index(rb),
            ra,
        )),
        I::Lfdux { frt, ra, rb } => one(update(
            fp_load(ea_x(state, ra, rb), 8, frt).base(ra).index(rb),
            ra,
        )),
        I::Stfs { frs, ra, imm } => one(store(VSU_MEM, ea_d(state, ra, imm), 4)
            .src(fpr(frs))
            .base(ra)),
        I::Stfd { frs, ra, imm } => one(store(VSU_MEM, ea_d(state, ra, imm), 8)
            .src(fpr(frs))
            .base(ra)),
        I::Stfsu { frs, ra, imm } => one(update(
            store(VSU_MEM, ea_d(state, ra, imm), 4)
                .src(fpr(frs))
                .base(ra),
            ra,
        )),
        I::Stfdu { frs, ra, imm } => one(update(
            store(VSU_MEM, ea_d(state, ra, imm), 8)
                .src(fpr(frs))
                .base(ra),
            ra,
        )),
        I::Stfiwx { frs, ra, rb } | I::Stfsx { frs, ra, rb } => {
            one(store(VSU_MEM, ea_x(state, ra, rb), 4)
                .src(fpr(frs))
                .base(ra)
                .index(rb))
        }
        I::Stfdx { frs, ra, rb } => one(store(VSU_MEM, ea_x(state, ra, rb), 8)
            .src(fpr(frs))
            .base(ra)
            .index(rb)),
        I::Stfsux { frs, ra, rb } => one(update(
            store(VSU_MEM, ea_x(state, ra, rb), 4)
                .src(fpr(frs))
                .base(ra)
                .index(rb),
            ra,
        )),
        I::Stfdux { frs, ra, rb } => one(update(
            store(VSU_MEM, ea_x(state, ra, rb), 8)
                .src(fpr(frs))
                .base(ra)
                .index(rb),
            ra,
        )),
        I::Fp63 {
            op,
            frt,
            fra,
            frb,
            frc,
            rc,
        } => one(fp63(op, frt, fra, frb, frc, rc)),
        I::Fp59 {
            op,
            frt,
            fra,
            frb,
            frc,
            rc,
        } => one(fp59(op, frt, fra, frb, frc, rc)),
        I::Li { rt, .. } | I::Lis { rt, .. } => one(fx(gpr(rt))),
        I::Mr { ra, rs }
        | I::Slwi { ra, rs, .. }
        | I::Srwi { ra, rs, .. }
        | I::Clrlwi { ra, rs, .. }
        | I::Clrldi { ra, rs, .. }
        | I::Sldi { ra, rs, .. }
        | I::Srdi { ra, rs, .. } => one(fx(gpr(ra)).src(gpr(rs))),
        I::Nop => one(Op::on(unit::FXU)),
        I::Fused(pair) => classify_fused(pair, state),
        I::Consumed => ([none, none], 0),
        I::Dcbz { ra, rb } => one(Op::on(unit::LSU)
            .kind(Kind::ZeroLine {
                ea: ea_x(state, ra, rb),
            })
            .base(ra)
            .index(rb)),
        // [CBE-Handbook p:145 s:6.1.6.1] the load-store unit translates the icbi address; the model
        // issues every cache-block instruction there.
        // [CBE-Handbook p:146 s:6.1.6.2] a TH(0) = '0' form other than
        // '0000' is implemented as '0000'; the TH(0) = '1' forms set up
        // or stop a data stream, which the model leaves out. In the
        // `ppu_issue_touch_forms` capture, a dcbt with TH = '1000' or
        // '1010' touches no line. A dcbt with TH = '0001' touches its line
        // as TH = '0000' does.
        I::Dcbt { ra, rb, th } if th & 0b1000 == 0 => one(Op::on(unit::LSU)
            .kind(Kind::Touch {
                ea: ea_x(state, ra, rb),
            })
            .base(ra)
            .index(rb)),
        // [CBE-Handbook p:145 s:6.1.6.2] dcbtst behaves as dcbt with TH = '0000'.
        // In the `ppu_issue_touch_forms` capture, a dcbtst with TH = '1010'
        // touches its line as TH = '0000' does.
        I::Dcbtst { ra, rb, .. } => one(Op::on(unit::LSU)
            .kind(Kind::Touch {
                ea: ea_x(state, ra, rb),
            })
            .base(ra)
            .index(rb)),
        I::Dcbf { ra, rb, .. } => one(Op::on(unit::LSU)
            .kind(Kind::Flush {
                ea: ea_x(state, ra, rb),
            })
            .base(ra)
            .index(rb)),
        I::Dcbst { ra, rb } => one(Op::on(unit::LSU)
            .kind(Kind::StoreBack {
                ea: ea_x(state, ra, rb),
            })
            .base(ra)
            .index(rb)),
        I::Icbi { ra, rb }
        | I::Dcbt { ra, rb, .. }
        | I::Dst { ra, rb, .. }
        | I::Dstst { ra, rb, .. } => one(Op::on(unit::LSU).base(ra).index(rb)),
        I::Dss { .. } => one(Op::on(unit::LSU)),
        I::Sc { lev } => one(Op::on(0).kind(Kind::Syscall {
            number: (lev == 0).then_some(state.gpr[11]),
        })),
        I::Sync { .. } => one(Op::on(unit::LSU).kind(Kind::Drain)),
        I::Eieio => one(Op::on(unit::LSU)),
        I::Isync => one(Op::on(unit::LSU).kind(Kind::Refetch)),
    }
}
