//! The per-instruction observer a host installs on a PPU unit.

use crate::instruction::PpuInstruction;
use crate::state::PpuState;

/// What a PPU unit reports to the observer the host installs with
/// [`crate::PpuExecutionUnit::set_tap`].
///
/// An implementor keeps its state behind interior mutability: a unit
/// and each of its clones share one instance.
pub trait PpuTap {
    /// `insn` is about to execute at `state.pc`.
    ///
    /// The unit calls this once per dispatch, before the instruction
    /// runs. The number of calls differs from the number of
    /// retirements in three cases:
    ///
    /// - A store finds the store buffer full: the unit calls again when
    ///   the store retries.
    /// - The second slot of a fused pair retires: the unit makes no call.
    /// - A step faults: the unit rewinds to its batch entry and retracts
    ///   no call.
    fn dispatch(&self, insn: &PpuInstruction, state: &PpuState);
}
