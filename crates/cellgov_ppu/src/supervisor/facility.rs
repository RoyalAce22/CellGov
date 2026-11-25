//! The MSR bits that make the floating-point and vector units available.

use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_FP, MSR_VXU};

use crate::exec::PpuInterrupt;
use crate::instruction::{Facility, PpuInstruction};
use crate::state::PpuState;

/// The unavailable interrupt that `insn` takes on `state`, or `None` when
/// `insn` can run.
///
/// A thread with no supervisor registers has no MSR, and runs every
/// instruction with both units available.
///
/// [PPC-Book3 p:72 s:5.5.10] An attempt to execute a floating-point instruction, a floating-point load or store included, with MSR\[FP\] = 0 takes the floating-point unavailable interrupt.
/// [CBE-Handbook p:258 s:9.5.15] An attempt to execute a VXU instruction with MSR\[VXU\] = 0 takes the VXU unavailable interrupt.
#[inline]
pub(crate) fn unavailable(insn: &PpuInstruction, state: &PpuState) -> Option<PpuInterrupt> {
    let msr = state.supervisor()?.msr;
    match insn.facility()? {
        Facility::FloatingPoint if msr & MSR_FP == 0 => Some(PpuInterrupt::FpUnavailable),
        Facility::Vector if msr & MSR_VXU == 0 => Some(PpuInterrupt::VxuUnavailable),
        Facility::FloatingPoint | Facility::Vector => None,
    }
}
