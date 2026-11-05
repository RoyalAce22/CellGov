//! The load-path probe a profiled unit runs on an instruction before it
//! executes.

use super::ppu_unit::PpuExecutionUnit;
use crate::instruction::{FusedPair, PpuInstruction};
use crate::profile::LoadOutcome;
use cellgov_mem::RegionView;

impl PpuExecutionUnit {
    /// The slot and outcome of the keyed load `insn` makes at `step_pc`,
    /// or `None` when `insn` makes no keyed load.
    ///
    /// The caller runs the probe before the load executes, so the probe
    /// reads the state and the store buffer that the load reads. The slot
    /// is `None` when the unit has no shadow slot at `step_pc`.
    pub(super) fn probe_load(
        &self,
        insn: &PpuInstruction,
        step_pc: u64,
        views: &[RegionView<'_>],
    ) -> Option<(Option<u64>, LoadOutcome)> {
        let (ra, imm, size) = keyed_load(insn)?;
        let ea = self.state.ea_d_form(ra, imm);
        let outcome = if self.store_buf.forward(ea, size).is_some() {
            LoadOutcome::Forwarded
        } else {
            // No view holds an access that runs past the top of the
            // address space. The load's own containment check agrees.
            let end = ea.checked_add(u64::from(size));
            views
                .iter()
                .position(|v| {
                    ea >= v.base && end.is_some_and(|e| e - v.base <= v.bytes.len() as u64)
                })
                .map_or(LoadOutcome::Unheld, LoadOutcome::Region)
        };
        let slot = self
            .instruction_shadow
            .as_ref()
            .filter(|s| s.base() <= step_pc && step_pc < s.end())
            .map(|_| step_pc);
        Some((slot, outcome))
    }
}

/// The base register, displacement and width of the load the profile
/// keys in `insn`: a D-form or DS-form scalar load, or the load half of
/// `lwz; cmpwi`, `lwz; mtlr` or `ld; mtlr`.
fn keyed_load(insn: &PpuInstruction) -> Option<(u8, i16, u8)> {
    match *insn {
        PpuInstruction::Fused(
            FusedPair::LwzCmpwi {
                ra_load, offset, ..
            }
            | FusedPair::LwzMtlr {
                ra_load, offset, ..
            },
        ) => Some((ra_load, offset, 4)),
        PpuInstruction::Fused(FusedPair::LdMtlr {
            ra_load, offset, ..
        }) => Some((ra_load, offset, 8)),
        _ => crate::exec::d_form_load(insn),
    }
}

/// Whether `insn` is a fused pair with a load half [`keyed_load`] does
/// not key.
pub(super) fn has_unkeyed_load(insn: &PpuInstruction) -> bool {
    matches!(
        insn,
        PpuInstruction::Fused(
            FusedPair::LdLd { .. }
                | FusedPair::LbzuCmpwi { .. }
                | FusedPair::LduLdu { .. }
                | FusedPair::LdRlwinm { .. }
                | FusedPair::LwzLwz { .. }
                | FusedPair::LdLdAny { .. }
                | FusedPair::LbzLbz { .. }
                | FusedPair::OrLwz { .. }
                | FusedPair::LdAddi { .. }
                | FusedPair::LwzOr { .. }
                | FusedPair::StdLwz { .. }
        )
    )
}
