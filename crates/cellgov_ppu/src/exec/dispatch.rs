//! Typed-variant dispatch for one decoded [`PpuInstruction`].

use cellgov_effects::Effect;
use cellgov_event::UnitId;

use crate::exec::verdict::ExecuteVerdict;
use crate::exec::{alu, branch, cr, mem, super_insn, vec};
use crate::fp;
use crate::instruction::ops::Fp63Op;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// Execute one decoded PPU instruction.
///
/// On [`ExecuteVerdict::BufferFull`] the caller must flush `store_buf`
/// and retry the same instruction.
pub fn execute(
    insn: &PpuInstruction,
    state: &mut PpuState,
    unit_id: UnitId,
    region_views: &[cellgov_mem::RegionView<'_>],
    effects: &mut Vec<Effect>,
    store_buf: &mut StoreBuffer,
) -> ExecuteVerdict {
    match *insn {
        PpuInstruction::Lwz { .. }
        | PpuInstruction::Lbz { .. }
        | PpuInstruction::Lhz { .. }
        | PpuInstruction::Lha { .. }
        | PpuInstruction::Lhau { .. }
        | PpuInstruction::Lmw { .. }
        | PpuInstruction::Lwzu { .. }
        | PpuInstruction::Lbzu { .. }
        | PpuInstruction::Lhzu { .. }
        | PpuInstruction::Ldu { .. }
        | PpuInstruction::Ld { .. }
        | PpuInstruction::Lwa { .. }
        | PpuInstruction::Lwzx { .. }
        | PpuInstruction::Lbzx { .. }
        | PpuInstruction::Ldx { .. }
        | PpuInstruction::Lhzx { .. }
        | PpuInstruction::Stw { .. }
        | PpuInstruction::Stb { .. }
        | PpuInstruction::Stbu { .. }
        | PpuInstruction::Stmw { .. }
        | PpuInstruction::Sth { .. }
        | PpuInstruction::Sthu { .. }
        | PpuInstruction::Std { .. }
        | PpuInstruction::Stwu { .. }
        | PpuInstruction::Stdu { .. }
        | PpuInstruction::Stwx { .. }
        | PpuInstruction::Stdx { .. }
        | PpuInstruction::Stdux { .. }
        | PpuInstruction::Stbx { .. }
        | PpuInstruction::Lwzux { .. }
        | PpuInstruction::Lbzux { .. }
        | PpuInstruction::Lhzux { .. }
        | PpuInstruction::Ldux { .. }
        | PpuInstruction::Lhax { .. }
        | PpuInstruction::Lhaux { .. }
        | PpuInstruction::Lwax { .. }
        | PpuInstruction::Lwaux { .. }
        | PpuInstruction::Sthx { .. }
        | PpuInstruction::Sthux { .. }
        | PpuInstruction::Stwux { .. }
        | PpuInstruction::Stbux { .. }
        | PpuInstruction::Lswi { .. }
        | PpuInstruction::Lswx { .. }
        | PpuInstruction::Stswi { .. }
        | PpuInstruction::Stswx { .. }
        | PpuInstruction::Ldbrx { .. }
        | PpuInstruction::Lwbrx { .. }
        | PpuInstruction::Lhbrx { .. }
        | PpuInstruction::Sdbrx { .. }
        | PpuInstruction::Stwbrx { .. }
        | PpuInstruction::Sthbrx { .. }
        | PpuInstruction::Ldarx { .. }
        | PpuInstruction::Stdcx { .. }
        | PpuInstruction::Lwarx { .. }
        | PpuInstruction::Stwcx { .. }
        | PpuInstruction::Lvlx { .. }
        | PpuInstruction::Lvrx { .. }
        | PpuInstruction::Lvlxl { .. }
        | PpuInstruction::Lvrxl { .. }
        | PpuInstruction::Lvsl { .. }
        | PpuInstruction::Lvebx { .. }
        | PpuInstruction::Lvsr { .. }
        | PpuInstruction::Lvehx { .. }
        | PpuInstruction::Lvewx { .. }
        | PpuInstruction::Lvx { .. }
        | PpuInstruction::Lvxl { .. }
        | PpuInstruction::Stvebx { .. }
        | PpuInstruction::Stvehx { .. }
        | PpuInstruction::Stvewx { .. }
        | PpuInstruction::Stvlx { .. }
        | PpuInstruction::Stvrx { .. }
        | PpuInstruction::Stvlxl { .. }
        | PpuInstruction::Stvrxl { .. }
        | PpuInstruction::Stvx { .. }
        | PpuInstruction::Stvxl { .. }
        | PpuInstruction::Lfs { .. }
        | PpuInstruction::Lfsu { .. }
        | PpuInstruction::Lfd { .. }
        | PpuInstruction::Lfdu { .. }
        | PpuInstruction::Stfs { .. }
        | PpuInstruction::Stfd { .. }
        | PpuInstruction::Stfsu { .. }
        | PpuInstruction::Stfdu { .. }
        | PpuInstruction::Stfiwx { .. }
        | PpuInstruction::Lfsx { .. }
        | PpuInstruction::Lfsux { .. }
        | PpuInstruction::Lfdx { .. }
        | PpuInstruction::Lfdux { .. }
        | PpuInstruction::Stfsx { .. }
        | PpuInstruction::Stfsux { .. }
        | PpuInstruction::Stfdx { .. }
        | PpuInstruction::Stfdux { .. }
        | PpuInstruction::Dcbz { .. }
        | PpuInstruction::Dcbst { .. }
        | PpuInstruction::Dcbf { .. }
        | PpuInstruction::Icbi { .. } => {
            mem::execute(insn, state, unit_id, region_views, effects, store_buf)
        }

        PpuInstruction::B { .. }
        | PpuInstruction::Bc { .. }
        | PpuInstruction::Bclr { .. }
        | PpuInstruction::Bcctr { .. } => branch::execute(insn, state),

        PpuInstruction::Mcrf { .. }
        | PpuInstruction::Crand { .. }
        | PpuInstruction::Crandc { .. }
        | PpuInstruction::Cror { .. }
        | PpuInstruction::Crorc { .. }
        | PpuInstruction::Crxor { .. }
        | PpuInstruction::Crnand { .. }
        | PpuInstruction::Crnor { .. }
        | PpuInstruction::Creqv { .. } => cr::execute(insn, state),

        PpuInstruction::Addi { .. }
        | PpuInstruction::Addis { .. }
        | PpuInstruction::Subfic { .. }
        | PpuInstruction::Mulli { .. }
        | PpuInstruction::Addic { .. }
        | PpuInstruction::AddicDot { .. }
        | PpuInstruction::Add { .. }
        | PpuInstruction::Subf { .. }
        | PpuInstruction::Subfc { .. }
        | PpuInstruction::Addc { .. }
        | PpuInstruction::Subfe { .. }
        | PpuInstruction::Neg { .. }
        | PpuInstruction::Mullw { .. }
        | PpuInstruction::Mulhwu { .. }
        | PpuInstruction::Mulhw { .. }
        | PpuInstruction::Mulhdu { .. }
        | PpuInstruction::Mulhd { .. }
        | PpuInstruction::Adde { .. }
        | PpuInstruction::Addze { .. }
        | PpuInstruction::Subfze { .. }
        | PpuInstruction::Subfme { .. }
        | PpuInstruction::Addme { .. }
        | PpuInstruction::Divw { .. }
        | PpuInstruction::Divwu { .. }
        | PpuInstruction::Divd { .. }
        | PpuInstruction::Divdu { .. }
        | PpuInstruction::Mulld { .. }
        | PpuInstruction::Or { .. }
        | PpuInstruction::Orc { .. }
        | PpuInstruction::And { .. }
        | PpuInstruction::Nor { .. }
        | PpuInstruction::Andc { .. }
        | PpuInstruction::Xor { .. }
        | PpuInstruction::Eqv { .. }
        | PpuInstruction::Nand { .. }
        | PpuInstruction::AndiDot { .. }
        | PpuInstruction::AndisDot { .. }
        | PpuInstruction::Slw { .. }
        | PpuInstruction::Srw { .. }
        | PpuInstruction::Srawi { .. }
        | PpuInstruction::Sraw { .. }
        | PpuInstruction::Srad { .. }
        | PpuInstruction::Sradi { .. }
        | PpuInstruction::Sld { .. }
        | PpuInstruction::Srd { .. }
        | PpuInstruction::Cntlzw { .. }
        | PpuInstruction::Cntlzd { .. }
        | PpuInstruction::Popcntb { .. }
        | PpuInstruction::Tw { .. }
        | PpuInstruction::Td { .. }
        | PpuInstruction::Twi { .. }
        | PpuInstruction::Tdi { .. }
        | PpuInstruction::Mcrxr { .. }
        | PpuInstruction::PpeIllegal { .. }
        | PpuInstruction::PpePrivileged { .. }
        | PpuInstruction::Extsh { .. }
        | PpuInstruction::Extsb { .. }
        | PpuInstruction::Extsw { .. }
        | PpuInstruction::Ori { .. }
        | PpuInstruction::Oris { .. }
        | PpuInstruction::Xori { .. }
        | PpuInstruction::Xoris { .. }
        | PpuInstruction::Cmpwi { .. }
        | PpuInstruction::Cmplwi { .. }
        | PpuInstruction::Cmpdi { .. }
        | PpuInstruction::Cmpldi { .. }
        | PpuInstruction::Cmpw { .. }
        | PpuInstruction::Cmplw { .. }
        | PpuInstruction::Cmpd { .. }
        | PpuInstruction::Cmpld { .. }
        | PpuInstruction::Mftb { .. }
        | PpuInstruction::Mftbu { .. }
        | PpuInstruction::Mfcr { .. }
        | PpuInstruction::Mtcrf { .. }
        | PpuInstruction::Mfocrf { .. }
        | PpuInstruction::Mtocrf { .. }
        | PpuInstruction::Mflr { .. }
        | PpuInstruction::Mtlr { .. }
        | PpuInstruction::Mfctr { .. }
        | PpuInstruction::Mtctr { .. }
        | PpuInstruction::Mfxer { .. }
        | PpuInstruction::Mtxer { .. }
        | PpuInstruction::Mfvrsave { .. }
        | PpuInstruction::Mfctrl { .. }
        | PpuInstruction::Mfsprg3 { .. }
        | PpuInstruction::Mtvrsave { .. }
        | PpuInstruction::Rlwinm { .. }
        | PpuInstruction::Rlwimi { .. }
        | PpuInstruction::Rlwnm { .. }
        | PpuInstruction::Rldicl { .. }
        | PpuInstruction::Rldicr { .. }
        | PpuInstruction::Rldic { .. }
        | PpuInstruction::Rldimi { .. }
        | PpuInstruction::Rldcl { .. }
        | PpuInstruction::Rldcr { .. } => alu::execute(insn, state),

        // [AltiVec-PEM p:6-177 s:6.2] vxor routes through the canonical
        // VX-form path so vector-pipeline hooks reach it alongside the
        // generic `Vx { op }` family.
        PpuInstruction::Vxor { vt, va, vb } => vec::execute_vx(
            state,
            crate::instruction::ops::VxOp::Vxor,
            false,
            vt,
            va,
            vb,
        ),
        PpuInstruction::Vx { op, rc, vt, va, vb } => vec::execute_vx(state, op, rc, vt, va, vb),
        PpuInstruction::Va { op, vt, va, vb, vc } => vec::execute_va(state, op, vt, va, vb, vc),
        PpuInstruction::Vsldoi { vt, va, vb, shb } => vec::execute_vsldoi(state, vt, va, vb, shb),
        PpuInstruction::Mfvscr { vt } => vec::execute_mfvscr(state, vt),
        PpuInstruction::Mtvscr { vb } => vec::execute_mtvscr(state, vb),

        PpuInstruction::Fp63 {
            op,
            frt,
            fra,
            frb,
            frc,
            rc,
        } => {
            let verdict = fp::execute_fp63(state, op, frt, fra, frb, frc);
            // The compares and mcrfs have no Rc field: bit 31 is reserved.
            let has_rc = !matches!(op, Fp63Op::Fcmpu | Fp63Op::Fcmpo | Fp63Op::Mcrfs);
            if rc && has_rc {
                fp::record_cr1(state);
            }
            verdict
        }
        PpuInstruction::Fp59 {
            op,
            frt,
            fra,
            frb,
            frc,
            rc,
        } => {
            let verdict = fp::execute_fp59(state, op, frt, fra, frb, frc);
            if rc {
                fp::record_cr1(state);
            }
            verdict
        }

        PpuInstruction::Li { .. }
        | PpuInstruction::Mr { .. }
        | PpuInstruction::Slwi { .. }
        | PpuInstruction::Srwi { .. }
        | PpuInstruction::Clrlwi { .. }
        | PpuInstruction::Nop
        | PpuInstruction::CmpwZero { .. }
        | PpuInstruction::CmpdZero { .. }
        | PpuInstruction::Clrldi { .. }
        | PpuInstruction::Sldi { .. }
        | PpuInstruction::Srdi { .. }
        | PpuInstruction::Lis { .. }
        | PpuInstruction::Bdnz { .. }
        | PpuInstruction::BcTrue { .. }
        | PpuInstruction::BcFalse { .. }
        | PpuInstruction::Blr { .. }
        | PpuInstruction::RlwinmMask { .. }
        | PpuInstruction::LwzCmpwi { .. }
        | PpuInstruction::LiStw { .. }
        | PpuInstruction::MflrStw { .. }
        | PpuInstruction::LwzMtlr { .. }
        | PpuInstruction::MflrStd { .. }
        | PpuInstruction::LdMtlr { .. }
        | PpuInstruction::StdStd { .. }
        | PpuInstruction::LdLd { .. }
        | PpuInstruction::LbzuCmpwi { .. }
        | PpuInstruction::OrAdd { .. }
        | PpuInstruction::LisOri { .. }
        | PpuInstruction::CmpwiBc { .. }
        | PpuInstruction::CmpwBc { .. }
        | PpuInstruction::CmpdiBc { .. }
        | PpuInstruction::CmpdBc { .. }
        | PpuInstruction::CmplwBc { .. }
        | PpuInstruction::Consumed => {
            super_insn::execute(insn, state, unit_id, region_views, effects, store_buf)
        }

        // [PPC-Book1 p:26 s:2.4.2] sc surfaces LEV to the system; service dispatch is runtime-defined.
        // LEV = 1 invokes the hypervisor; see `PpuInterrupt::SystemCall`.
        PpuInstruction::Sc { lev: 1 } => ExecuteVerdict::Fault(super::PpuFault::Interrupt(
            super::PpuInterrupt::SystemCall { lev: 1 },
        )),
        PpuInstruction::Sc { lev } => ExecuteVerdict::Syscall { lev },

        // One unit's accesses run in program order and each step's
        // stores commit as one batch, so no barrier changes a result.
        // The unit records where each barrier retires.
        // [PPC-Book2 p:26 s:3.3.3] a memory barrier orders storage accesses.
        // [PPC-Book2 p:27 s:3.3.3] sync alters no special register; [PPC-Book2 p:28 s:3.3.3] nor does eieio.
        // [PPC-Book2 p:22 s:3.3.1] isync alters no special register.
        PpuInstruction::Sync { .. } | PpuInstruction::Eieio | PpuInstruction::Isync => {
            ExecuteVerdict::Continue
        }

        // [PPC-Book2 p:19 s:3.2.2] a touch hint never invokes the data storage handler.
        // [CBE-Handbook p:147 s:6.1.6.6] dst, dstst and dss are no-ops on the CBE.
        PpuInstruction::Dcbt { .. }
        | PpuInstruction::Dcbtst { .. }
        | PpuInstruction::Dst { .. }
        | PpuInstruction::Dstst { .. }
        | PpuInstruction::Dss { .. } => ExecuteVerdict::Continue,
    }
}
