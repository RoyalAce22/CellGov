//! The class predicates of a decoded instruction: super-pair,
//! barrier and decode-only stub.

use super::PpuInstruction;

impl PpuInstruction {
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
