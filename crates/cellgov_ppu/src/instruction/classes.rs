//! The class predicates of a decoded instruction: super-pair,
//! barrier, decode-only stub, facility and branch.

use super::PpuInstruction;

#[cfg(test)]
#[path = "tests/facility_tests.rs"]
mod facility_tests;

/// A unit that an MSR bit makes available to an instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Facility {
    /// The floating-point unit, which MSR\[FP\] makes available.
    FloatingPoint,
    /// The vector unit, which MSR\[VXU\] makes available.
    Vector,
}

impl PpuInstruction {
    /// The facility that this instruction needs, or `None` for an
    /// instruction that needs neither.
    ///
    /// [PPC-Book3 p:10 s:2.2.3] With MSR\[FP\] = 0 the processor executes no floating-point instruction, floating-point loads, stores and moves included.
    /// [CBE-Handbook p:740 s:A.3.2] Table A-8: the vector loads include `lvsl` and `lvsr`, and the PPE runs `dst`, `dstst` and `dss` as no-ops.
    /// [AltiVec-PEM p:2-9 s:2.5] The data stream instructions and the VRSAVE moves run with the vector unit disabled.
    pub(crate) const fn facility(&self) -> Option<Facility> {
        use PpuInstruction as I;
        match self {
            I::Lfs { .. }
            | I::Lfsu { .. }
            | I::Lfd { .. }
            | I::Lfdu { .. }
            | I::Lfsx { .. }
            | I::Lfsux { .. }
            | I::Lfdx { .. }
            | I::Lfdux { .. }
            | I::Stfs { .. }
            | I::Stfsu { .. }
            | I::Stfd { .. }
            | I::Stfdu { .. }
            | I::Stfsx { .. }
            | I::Stfsux { .. }
            | I::Stfdx { .. }
            | I::Stfdux { .. }
            | I::Stfiwx { .. }
            | I::Fp59 { .. }
            | I::Fp63 { .. } => Some(Facility::FloatingPoint),
            I::Vx { .. }
            | I::Va { .. }
            | I::Vxor { .. }
            | I::Vsldoi { .. }
            | I::Mfvscr { .. }
            | I::Mtvscr { .. }
            | I::Lvsl { .. }
            | I::Lvsr { .. }
            | I::Lvebx { .. }
            | I::Lvehx { .. }
            | I::Lvewx { .. }
            | I::Lvx { .. }
            | I::Lvxl { .. }
            | I::Lvlx { .. }
            | I::Lvlxl { .. }
            | I::Lvrx { .. }
            | I::Lvrxl { .. }
            | I::Stvebx { .. }
            | I::Stvehx { .. }
            | I::Stvewx { .. }
            | I::Stvx { .. }
            | I::Stvxl { .. }
            | I::Stvlx { .. }
            | I::Stvlxl { .. }
            | I::Stvrx { .. }
            | I::Stvrxl { .. } => Some(Facility::Vector),
            _ => None,
        }
    }

    /// Whether this instruction is a branch: `b`, `bc`, `bclr`, `bcctr`,
    /// or a quickening of one.
    ///
    /// A fused pair is never a branch here. Only a thread with supervisor
    /// registers traces branches, and such a thread runs no fused pair.
    pub(crate) const fn is_branch(&self) -> bool {
        matches!(
            self,
            PpuInstruction::B { .. }
                | PpuInstruction::Bc { .. }
                | PpuInstruction::Bclr { .. }
                | PpuInstruction::Bcctr { .. }
                | PpuInstruction::Bdnz { .. }
                | PpuInstruction::BcTrue { .. }
                | PpuInstruction::BcFalse { .. }
                | PpuInstruction::Blr { .. }
        )
    }

    /// Whether this instruction is a fused 2-instruction super-pair
    /// requiring a [`PpuInstruction::Consumed`] placeholder at PC+4.
    /// One-instruction quickenings are not super-pairs.
    #[inline]
    pub fn is_super_pair(&self) -> bool {
        matches!(self, PpuInstruction::Fused(_))
    }

    /// Whether this is a fused pair whose second half reads memory after
    /// its first half accessed memory.
    pub const fn reads_after_access(&self) -> bool {
        use super::FusedPair as F;
        let PpuInstruction::Fused(pair) = self else {
            return false;
        };
        match pair {
            F::LdLd { .. }
            | F::LduLdu { .. }
            | F::LwzLwz { .. }
            | F::StdLwz { .. }
            | F::LdLdAny { .. }
            | F::LbzLbz { .. } => true,
            F::LwzCmpwi { .. }
            | F::LiStw { .. }
            | F::MflrStw { .. }
            | F::LwzMtlr { .. }
            | F::MflrStd { .. }
            | F::LdMtlr { .. }
            | F::StdStd { .. }
            | F::LbzuCmpwi { .. }
            | F::CmpdNor { .. }
            | F::ExtswBlr { .. }
            | F::OrAdd { .. }
            | F::LisOri { .. }
            | F::CmpwiBc { .. }
            | F::CmpwBc { .. }
            | F::CmpdiBc { .. }
            | F::CmpdBc { .. }
            | F::CmplwBc { .. }
            | F::OrRlwinm { .. }
            | F::LdRlwinm { .. }
            | F::AddiBc { .. }
            | F::OrB { .. }
            | F::MtlrAddi { .. }
            | F::RldiclRldicl { .. }
            | F::OriAnd { .. }
            | F::RldicrOris { .. }
            | F::OrOr { .. }
            | F::OrBc { .. }
            | F::OrLwz { .. }
            | F::OrBctrl { .. }
            | F::LdAddi { .. }
            | F::LwzOr { .. } => false,
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
