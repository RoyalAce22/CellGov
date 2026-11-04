//! Guest fault codes the PPU unit yields in `FaultKind::Guest`.

use strum::VariantArray;

use crate::instruction::PpuInstructionKind;

/// PPU tried to fetch at an address beyond guest memory.
pub const FAULT_PC_OUT_OF_RANGE: u32 = 0x0102_0000;
/// Instruction word did not match any implemented encoding.
pub const FAULT_DECODE_ERROR: u32 = 0x0105_0000;
/// Load or store targeted an out-of-bounds guest address.
pub const FAULT_INVALID_ADDRESS: u32 = 0x0106_0000;
/// Syscall number has no handler.
pub const FAULT_UNSUPPORTED_SYSCALL: u32 = 0x0107_0000;
/// Debug breakpoint fired at a user-requested PC.
pub const FAULT_DEBUG_BREAK: u32 = 0x0108_0000;
/// Decoded instruction with no exec arm. The detail half names its kind
/// ([`unimplemented_kind`]).
pub const FAULT_UNIMPLEMENTED_INSN: u32 = 0x0109_0000;
/// Program trap fired (e.g. `tw` / `td` with a TO-selected
/// condition met).
pub const FAULT_PROGRAM_TRAP: u32 = 0x010A_0000;
/// Reservation operand EA not aligned to operand size.
pub const FAULT_ALIGNMENT_INTERRUPT: u32 = 0x010B_0000;
/// Instruction encoded in an invalid form.
pub const FAULT_INVALID_FORM: u32 = 0x010C_0000;
/// Defined PowerPC instruction the PPE does not implement.
pub const FAULT_ILLEGAL_INSTRUCTION: u32 = 0x010D_0000;
/// Instruction the PPE runs only in privileged or hypervisor state.
pub const FAULT_PRIVILEGED_INSTRUCTION: u32 = 0x010E_0000;

/// True when `code` belongs to the [`FAULT_DECODE_ERROR`] class.
#[inline]
pub fn is_decode_error(code: u32) -> bool {
    (code & 0xFFFF_0000) == FAULT_DECODE_ERROR
}

/// The index a [`FAULT_UNIMPLEMENTED_INSN`] code carries for `kind`: its
/// position in [`PpuInstructionKind::VARIANTS`].
pub(crate) fn kind_index(kind: PpuInstructionKind) -> u16 {
    kind as u16
}

/// The instruction kind a [`FAULT_UNIMPLEMENTED_INSN`] code names, or
/// `None` for a code of another class or an index past the last kind.
pub fn unimplemented_kind(code: u32) -> Option<PpuInstructionKind> {
    if code & 0xFFFF_0000 != FAULT_UNIMPLEMENTED_INSN {
        return None;
    }
    PpuInstructionKind::VARIANTS
        .get((code & 0xFFFF) as usize)
        .copied()
}

#[cfg(test)]
#[path = "tests/fault_codes_tests.rs"]
mod tests;
