//! The per-instruction observer a host installs on a PPU unit.

use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_event::UnitId;

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
    /// retirements in four cases:
    ///
    /// - A store finds the store buffer full: the unit calls again when
    ///   the store retries.
    /// - The second slot of a fused pair retires: the unit makes no call.
    /// - A fused pair faults: the unit calls again when its first
    ///   instruction runs alone.
    /// - A step faults behind retired instructions: the unit calls again
    ///   when the next batch runs it. The unit retracts no call.
    fn dispatch(&self, unit: UnitId, insn: &PpuInstruction, state: &PpuState);
}
