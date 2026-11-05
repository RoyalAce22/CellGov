//! The class predicates of a decoded instruction: super-pair,
//! barrier and decode-only stub.

use super::PpuInstruction;

impl PpuInstruction {
    /// Whether this instruction is a fused 2-instruction super-pair
    /// requiring a [`PpuInstruction::Consumed`] placeholder at PC+4.
    /// One-instruction quickenings (`Li`, `Slwi`, `Srwi`, `Clrlwi`)
    /// are not super-pairs.
    ///
    /// Exhaustive: every variant must declare its super-pair status
    /// at compile time. Producer side lives in
    /// `crate::shadow::superpair`; the fused variants emitted there
    /// must match the variants returning `true` here. A divergence
    /// causes silent re-execution of the second half of the pair: the
    /// fetch loop runs the word stored at the `Consumed` slot as well.
    pub fn is_super_pair(&self) -> bool {
        match self {
            PpuInstruction::LwzCmpwi { .. }
            | PpuInstruction::LiStw { .. }
            | PpuInstruction::MflrStw { .. }
            | PpuInstruction::LwzMtlr { .. }
            | PpuInstruction::MflrStd { .. }
            | PpuInstruction::LdMtlr { .. }
            | PpuInstruction::StdStd { .. }
            | PpuInstruction::CmpwiBc { .. }
            | PpuInstruction::CmpwBc { .. } => true,
            PpuInstruction::PpeIllegal { .. }
            | PpuInstruction::PpePrivileged { .. }
            | PpuInstruction::Lwz { .. }
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
            | PpuInstruction::Stw { .. }
            | PpuInstruction::Stwu { .. }
            | PpuInstruction::Stdu { .. }
            | PpuInstruction::Stb { .. }
            | PpuInstruction::Stbu { .. }
            | PpuInstruction::Stmw { .. }
            | PpuInstruction::Sth { .. }
            | PpuInstruction::Sthu { .. }
            | PpuInstruction::Std { .. }
            | PpuInstruction::Addi { .. }
            | PpuInstruction::Addis { .. }
            | PpuInstruction::Subfic { .. }
            | PpuInstruction::Mulli { .. }
            | PpuInstruction::Addic { .. }
            | PpuInstruction::AddicDot { .. }
            | PpuInstruction::Add { .. }
            | PpuInstruction::Or { .. }
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
            | PpuInstruction::Mulld { .. }
            | PpuInstruction::Ldarx { .. }
            | PpuInstruction::Stdcx { .. }
            | PpuInstruction::Lwarx { .. }
            | PpuInstruction::Stwcx { .. }
            | PpuInstruction::Xori { .. }
            | PpuInstruction::Xoris { .. }
            | PpuInstruction::Divw { .. }
            | PpuInstruction::Divwu { .. }
            | PpuInstruction::Divd { .. }
            | PpuInstruction::Divdu { .. }
            | PpuInstruction::And { .. }
            | PpuInstruction::Andc { .. }
            | PpuInstruction::Nor { .. }
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
            | PpuInstruction::Orc { .. }
            | PpuInstruction::Extsh { .. }
            | PpuInstruction::Extsb { .. }
            | PpuInstruction::Extsw { .. }
            | PpuInstruction::Ori { .. }
            | PpuInstruction::Oris { .. }
            | PpuInstruction::Cmpwi { .. }
            | PpuInstruction::Cmplwi { .. }
            | PpuInstruction::Cmpdi { .. }
            | PpuInstruction::Cmpldi { .. }
            | PpuInstruction::Cmpw { .. }
            | PpuInstruction::Cmplw { .. }
            | PpuInstruction::Cmpd { .. }
            | PpuInstruction::Cmpld { .. }
            | PpuInstruction::B { .. }
            | PpuInstruction::Bc { .. }
            | PpuInstruction::Bclr { .. }
            | PpuInstruction::Bcctr { .. }
            | PpuInstruction::Mcrf { .. }
            | PpuInstruction::Crand { .. }
            | PpuInstruction::Crandc { .. }
            | PpuInstruction::Cror { .. }
            | PpuInstruction::Crorc { .. }
            | PpuInstruction::Crxor { .. }
            | PpuInstruction::Crnand { .. }
            | PpuInstruction::Crnor { .. }
            | PpuInstruction::Creqv { .. }
            | PpuInstruction::Lwzx { .. }
            | PpuInstruction::Lbzx { .. }
            | PpuInstruction::Ldx { .. }
            | PpuInstruction::Lhzx { .. }
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
            | PpuInstruction::Rldcr { .. }
            | PpuInstruction::Vx { .. }
            | PpuInstruction::Va { .. }
            | PpuInstruction::Vxor { .. }
            | PpuInstruction::Vsldoi { .. }
            | PpuInstruction::Mfvscr { .. }
            | PpuInstruction::Mtvscr { .. }
            | PpuInstruction::Lvlx { .. }
            | PpuInstruction::Lvrx { .. }
            | PpuInstruction::Lvlxl { .. }
            | PpuInstruction::Lvrxl { .. }
            | PpuInstruction::Stvlx { .. }
            | PpuInstruction::Stvrx { .. }
            | PpuInstruction::Stvlxl { .. }
            | PpuInstruction::Stvrxl { .. }
            | PpuInstruction::Lvsl { .. }
            | PpuInstruction::Lvebx { .. }
            | PpuInstruction::Lvsr { .. }
            | PpuInstruction::Lvehx { .. }
            | PpuInstruction::Lvewx { .. }
            | PpuInstruction::Lvx { .. }
            | PpuInstruction::Stvebx { .. }
            | PpuInstruction::Stvehx { .. }
            | PpuInstruction::Stvewx { .. }
            | PpuInstruction::Lvxl { .. }
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
            | PpuInstruction::Fp63 { .. }
            | PpuInstruction::Fp59 { .. }
            | PpuInstruction::Li { .. }
            | PpuInstruction::Mr { .. }
            | PpuInstruction::Slwi { .. }
            | PpuInstruction::Srwi { .. }
            | PpuInstruction::Clrlwi { .. }
            | PpuInstruction::Nop
            | PpuInstruction::CmpwZero { .. }
            | PpuInstruction::Clrldi { .. }
            | PpuInstruction::Sldi { .. }
            | PpuInstruction::Srdi { .. }
            | PpuInstruction::Lis { .. }
            | PpuInstruction::Consumed
            | PpuInstruction::Dcbz { .. }
            | PpuInstruction::Dcbst { .. }
            | PpuInstruction::Dcbf { .. }
            | PpuInstruction::Icbi { .. }
            | PpuInstruction::Dcbt { .. }
            | PpuInstruction::Dcbtst { .. }
            | PpuInstruction::Dst { .. }
            | PpuInstruction::Dstst { .. }
            | PpuInstruction::Dss { .. }
            | PpuInstruction::Sc { .. }
            | PpuInstruction::Sync { .. }
            | PpuInstruction::Eieio
            | PpuInstruction::Isync => false,
        }
    }

    /// The barrier this instruction is, if it is one.
    pub const fn barrier_kind(&self) -> Option<cellgov_exec::BarrierKind> {
        use cellgov_exec::BarrierKind;
        match self {
            PpuInstruction::Sync { l } => Some(BarrierKind::ppu_sync(*l)),
            PpuInstruction::Eieio => Some(BarrierKind::Eieio),
            PpuInstruction::Isync => Some(BarrierKind::Isync),
            _ => None,
        }
    }

    /// Whether this variant is a decode-only stub: the decoder
    /// produced it (so the scan counts it as "accepted") but the
    /// executor will fault via `UnimplementedInstruction` for at
    /// least some sub-opcodes. New typed variants are not stubs by
    /// default; the predicate names the catch-all family-dispatch
    /// variants explicitly.
    ///
    /// Used by the prescan to keep stubs out of `words_accepted` --
    /// counting a "decodes but doesn't execute" word as accepted
    /// under-reports the gap surface. See
    /// [`crate::prescan::PrescanReport::words_stubbed`].
    pub fn is_stub_variant(&self) -> bool {
        matches!(
            self,
            PpuInstruction::Vx { .. }
                | PpuInstruction::Va { .. }
                | PpuInstruction::Fp59 { .. }
                | PpuInstruction::Fp63 { .. }
        )
    }
}
