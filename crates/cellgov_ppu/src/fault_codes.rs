//! Guest fault codes the PPU unit yields in `FaultKind::Guest`.
//!
//! The high half names the class: one prefix per PPE interrupt, and one
//! per CellGov stop. The low half holds the class's detail.

use strum::VariantArray;

use crate::instruction::PpuInstructionKind;

/// Instruction storage interrupt. The detail half is SRR1 bits 32:47.
pub const FAULT_INSTRUCTION_STORAGE: u32 = 0x0102_0000;
/// Data storage interrupt. The detail half is DSISR bits 0:15.
pub const FAULT_DATA_STORAGE: u32 = 0x0106_0000;
/// CellGov stop: a debug breakpoint fired at a user-requested PC.
pub const FAULT_DEBUG_BREAK: u32 = 0x0108_0000;
/// CellGov stop: a decoded instruction with no execution arm. The detail
/// half names its kind ([`unimplemented_kind`]).
pub const FAULT_UNIMPLEMENTED_INSN: u32 = 0x0109_0000;
/// Program interrupt. The detail half's high byte is SRR1 bits 40:47;
/// for a trap, its low byte is TO.
pub const FAULT_PROGRAM: u32 = 0x010A_0000;
/// Alignment interrupt.
pub const FAULT_ALIGNMENT: u32 = 0x010B_0000;
/// System call interrupt with LEV = 1. The detail half is LEV.
pub const FAULT_SYSTEM_CALL: u32 = 0x010F_0000;
/// Data segment interrupt. The detail half is DSISR bits 0:15.
pub const FAULT_DATA_SEGMENT: u32 = 0x0110_0000;
/// Instruction segment interrupt.
pub const FAULT_INSTRUCTION_SEGMENT: u32 = 0x0111_0000;
/// Floating-point unavailable interrupt.
pub const FAULT_FP_UNAVAILABLE: u32 = 0x0112_0000;
/// VXU unavailable interrupt.
pub const FAULT_VXU_UNAVAILABLE: u32 = 0x0113_0000;
/// CellGov stop: a state the interpreter's own invariants exclude.
pub const FAULT_INTERNAL: u32 = 0x0114_0000;

/// The class half of a fault code.
const CLASS_MASK: u32 = 0xFFFF_0000;

/// The class a PPU fault code names: a PPE interrupt, with the program
/// interrupt split by its reason, or a CellGov stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, VariantArray)]
pub enum PpuFaultClass {
    /// Instruction storage interrupt.
    InstructionStorage,
    /// Instruction segment interrupt.
    InstructionSegment,
    /// Data storage interrupt.
    DataStorage,
    /// Data segment interrupt.
    DataSegment,
    /// Alignment interrupt.
    Alignment,
    /// Floating-point enabled program interrupt.
    ProgramFpEnabled,
    /// Illegal-instruction program interrupt.
    ProgramIllegal,
    /// Privileged-instruction program interrupt.
    ProgramPrivileged,
    /// Trap program interrupt.
    ProgramTrap,
    /// Floating-point unavailable interrupt.
    FpUnavailable,
    /// VXU unavailable interrupt.
    VxuUnavailable,
    /// System call interrupt with LEV = 1.
    SystemCall,
    /// CellGov stop: a decoded instruction with no execution arm.
    Unimplemented,
    /// CellGov stop: a debug breakpoint.
    DebugBreak,
    /// CellGov stop: a state the interpreter's invariants exclude.
    Internal,
}

impl PpuFaultClass {
    /// The class `code` names, or `None` for a code no PPU fault yields.
    pub fn of_code(code: u32) -> Option<Self> {
        let class = match code & CLASS_MASK {
            FAULT_INSTRUCTION_STORAGE => Self::InstructionStorage,
            FAULT_INSTRUCTION_SEGMENT => Self::InstructionSegment,
            FAULT_DATA_STORAGE => Self::DataStorage,
            FAULT_DATA_SEGMENT => Self::DataSegment,
            FAULT_ALIGNMENT => Self::Alignment,
            FAULT_PROGRAM => return program_reason(code),
            FAULT_FP_UNAVAILABLE => Self::FpUnavailable,
            FAULT_VXU_UNAVAILABLE => Self::VxuUnavailable,
            FAULT_SYSTEM_CALL => Self::SystemCall,
            FAULT_UNIMPLEMENTED_INSN => Self::Unimplemented,
            FAULT_DEBUG_BREAK => Self::DebugBreak,
            FAULT_INTERNAL => Self::Internal,
            _ => return None,
        };
        Some(class)
    }

    /// Whether the class is a PPE interrupt; the other classes are CellGov
    /// stops.
    pub fn is_interrupt(self) -> bool {
        !matches!(
            self,
            Self::Unimplemented | Self::DebugBreak | Self::Internal
        )
    }
}

/// The program-interrupt reason a [`FAULT_PROGRAM`] code's detail names.
/// A code a fault yields sets exactly one reason bit.
fn program_reason(code: u32) -> Option<PpuFaultClass> {
    use cellgov_ps3_abi::hw::ppu_interrupt as irq;
    let reason = u64::from((code >> 8) & 0xFF) << 16;
    match reason {
        irq::SRR1_PROGRAM_FP_ENABLED => Some(PpuFaultClass::ProgramFpEnabled),
        irq::SRR1_PROGRAM_ILLEGAL => Some(PpuFaultClass::ProgramIllegal),
        irq::SRR1_PROGRAM_PRIVILEGED => Some(PpuFaultClass::ProgramPrivileged),
        irq::SRR1_PROGRAM_TRAP => Some(PpuFaultClass::ProgramTrap),
        _ => None,
    }
}

/// The index a [`FAULT_UNIMPLEMENTED_INSN`] code carries for `kind`: its
/// position in [`PpuInstructionKind::VARIANTS`].
pub(crate) fn kind_index(kind: PpuInstructionKind) -> u16 {
    kind as u16
}

/// The instruction kind a [`FAULT_UNIMPLEMENTED_INSN`] code names, or
/// `None` for a code of another class or an index past the last kind.
pub fn unimplemented_kind(code: u32) -> Option<PpuInstructionKind> {
    if code & CLASS_MASK != FAULT_UNIMPLEMENTED_INSN {
        return None;
    }
    PpuInstructionKind::VARIANTS
        .get((code & 0xFFFF) as usize)
        .copied()
}

#[cfg(test)]
#[path = "tests/fault_codes_tests.rs"]
mod tests;
