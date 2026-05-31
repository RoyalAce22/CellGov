//! [`execute`] -- typed-variant dispatch for one decoded
//! [`PpuInstruction`]. Each form routes to the matching sibling
//! submodule (`alu`, `branch`, `cr`, `mem`, `super_insn`, `vec`).

use cellgov_effects::Effect;
use cellgov_event::UnitId;

use crate::exec::verdict::ExecuteVerdict;
use crate::exec::{alu, branch, cr, mem, super_insn, vec};
use crate::fp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// Execute one decoded PPU instruction.
///
/// Caller flushes `store_buf` at block boundaries; on
/// [`ExecuteVerdict::BufferFull`] the same instruction must be
/// retried after a flush.
pub fn execute(
    insn: &PpuInstruction,
    state: &mut PpuState,
    unit_id: UnitId,
    region_views: &[(u64, &[u8])],
    effects: &mut Vec<Effect>,
    store_buf: &mut StoreBuffer,
) -> ExecuteVerdict {
    cellgov_mem::store_watch::set_last_ppu_cia(state.pc as u32);
    crate::hle_watch::on_dispatch(state.pc as u32, &state.gpr, state.lr);
    match *insn {
        PpuInstruction::Sc { .. } => crate::hle_watch::on_syscall(state.pc as u32, &state.gpr),
        PpuInstruction::B {
            offset,
            aa,
            link: true,
        } => {
            let target = if aa {
                offset as u32
            } else {
                (state.pc as i32).wrapping_add(offset) as u32
            };
            crate::hle_watch::on_branch_link(state.pc as u32, &state.gpr, target);
        }
        PpuInstruction::Bcctr { link: true, .. } => {
            crate::hle_watch::on_branch_link(state.pc as u32, &state.gpr, state.ctr as u32);
        }
        PpuInstruction::Bclr { link: true, .. } => {
            crate::hle_watch::on_branch_link(state.pc as u32, &state.gpr, state.lr as u32);
        }
        _ => {}
    }
    match *insn {
        PpuInstruction::Lwz { .. }
        | PpuInstruction::Lbz { .. }
        | PpuInstruction::Lhz { .. }
        | PpuInstruction::Lha { .. }
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
        | PpuInstruction::Sth { .. }
        | PpuInstruction::Sthu { .. }
        | PpuInstruction::Std { .. }
        | PpuInstruction::Stwu { .. }
        | PpuInstruction::Stdu { .. }
        | PpuInstruction::Stwx { .. }
        | PpuInstruction::Stdx { .. }
        | PpuInstruction::Stdux { .. }
        | PpuInstruction::Stbx { .. }
        | PpuInstruction::Ldarx { .. }
        | PpuInstruction::Stdcx { .. }
        | PpuInstruction::Lwarx { .. }
        | PpuInstruction::Stwcx { .. }
        | PpuInstruction::Lvlx { .. }
        | PpuInstruction::Lvrx { .. }
        | PpuInstruction::Stvx { .. }
        | PpuInstruction::Lfs { .. }
        | PpuInstruction::Lfd { .. }
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
        | PpuInstruction::Dcbz { .. } => {
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
        | PpuInstruction::Subfe { .. }
        | PpuInstruction::Neg { .. }
        | PpuInstruction::Mullw { .. }
        | PpuInstruction::Mulhwu { .. }
        | PpuInstruction::Mulhw { .. }
        | PpuInstruction::Mulhdu { .. }
        | PpuInstruction::Mulhd { .. }
        | PpuInstruction::Adde { .. }
        | PpuInstruction::Addze { .. }
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
        | PpuInstruction::Mflr { .. }
        | PpuInstruction::Mtlr { .. }
        | PpuInstruction::Mfctr { .. }
        | PpuInstruction::Mtctr { .. }
        | PpuInstruction::Rlwinm { .. }
        | PpuInstruction::Rlwimi { .. }
        | PpuInstruction::Rlwnm { .. }
        | PpuInstruction::Rldicl { .. }
        | PpuInstruction::Rldicr { .. }
        | PpuInstruction::Rldic { .. }
        | PpuInstruction::Rldimi { .. } => alu::execute(insn, state),

        PpuInstruction::Vxor { vt, va, vb } => {
            let va = state.vr[va as usize];
            let vb = state.vr[vb as usize];
            state.vr[vt as usize] = va ^ vb;
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-21] lvx EA = (rA|0)+rB masked with ~0xF; loads 16 bytes.
        PpuInstruction::Vx {
            xo: 103,
            vt,
            va,
            vb,
        } => mem::execute_lvx(state, vt, va, vb, region_views, store_buf),
        PpuInstruction::Vx { xo, vt, va, vb } => vec::execute_vx(state, xo, vt, va, vb),
        PpuInstruction::Va { xo, vt, va, vb, vc } => vec::execute_va(state, xo, vt, va, vb, vc),
        PpuInstruction::Vsldoi { vt, va, vb, shb } => vec::execute_vsldoi(state, vt, va, vb, shb),

        // TODO(fp-rc): record-form CR1 update pending FPSCR plumbing;
        // `_rc` is preserved at decode.
        PpuInstruction::Fp63 {
            xo,
            frt,
            fra,
            frb,
            frc,
            rc: _rc,
        } => fp::execute_fp63(state, xo, frt, fra, frb, frc),
        PpuInstruction::Fp59 {
            xo,
            frt,
            fra,
            frb,
            frc,
            rc: _rc,
        } => fp::execute_fp59(state, xo, frt, fra, frb, frc),

        PpuInstruction::Li { .. }
        | PpuInstruction::Mr { .. }
        | PpuInstruction::Slwi { .. }
        | PpuInstruction::Srwi { .. }
        | PpuInstruction::Clrlwi { .. }
        | PpuInstruction::Nop
        | PpuInstruction::CmpwZero { .. }
        | PpuInstruction::Clrldi { .. }
        | PpuInstruction::Sldi { .. }
        | PpuInstruction::Srdi { .. }
        | PpuInstruction::LwzCmpwi { .. }
        | PpuInstruction::LiStw { .. }
        | PpuInstruction::MflrStw { .. }
        | PpuInstruction::LwzMtlr { .. }
        | PpuInstruction::MflrStd { .. }
        | PpuInstruction::LdMtlr { .. }
        | PpuInstruction::StdStd { .. }
        | PpuInstruction::CmpwiBc { .. }
        | PpuInstruction::CmpwBc { .. }
        | PpuInstruction::Consumed => super_insn::execute(insn, state, region_views, store_buf),

        // [PPC-Book1 p:26 s:2.4.2] sc surfaces LEV to the system; service dispatch is runtime-defined.
        PpuInstruction::Sc { lev } => ExecuteVerdict::Syscall { lev },
    }
}
