//! PPU-specific architectural fault categories.

/// PPU-specific fault categories.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PpuFault {
    /// PC outside addressable memory.
    #[error("PPU PC out of range at 0x{0:016x}")]
    PcOutOfRange(u64),
    /// Invalid memory access address.
    #[error("PPU invalid address at 0x{0:016x}")]
    InvalidAddress(u64),
    /// Unsupported syscall number.
    #[error("PPU unsupported syscall {0}")]
    UnsupportedSyscall(u64),
    /// Decoded instruction had no execution arm; payload is the
    /// offending sub-opcode.
    #[error("PPU unimplemented instruction sub-opcode 0x{0:x}")]
    UnimplementedInstruction(u64),
    // [PPC-Book1 p:62 s:3.3.10 Fixed-Point Trap Instructions] tw/td invoke the system trap handler when any TO-selected condition holds.
    /// Program trap fired (e.g. `tw` / `td` with a TO-selected
    /// condition met). Payload is the TO field.
    #[error("PPU program trap (TO=0x{0:02x})")]
    ProgramTrap(u8),
    // [PPC-Book2 p:24 s:3.3] lwarx/ldarx: "EA must be a multiple of [4/8]"; misaligned raises an alignment interrupt.
    // [PPC-Book2 p:25 s:3.3] stwcx./stdcx.: same alignment contract; the architecture permits either the alignment error handler or boundedly undefined results. CellGov takes the handler arm so a misaligned reservation is a named fault instead of a silent divergence.
    /// Reservation operand (`lwarx` / `ldarx` / `stwcx.` / `stdcx.`)
    /// EA not aligned to the operand size (4 or 8 bytes). Payload
    /// is the misaligned EA.
    #[error("PPU alignment interrupt on misaligned reservation EA 0x{0:016x}")]
    AlignmentInterrupt(u64),
    // [PPC-Book1 p:13 s:1.9.2] An invalid form either invokes the system illegal instruction error handler or yields boundedly undefined results. CellGov takes the handler arm so both build profiles refuse the form the same way.
    // [CBE-Handbook p:254 s:9.5.9] The PPE takes an illegal-instruction program interrupt for a load or store with update in an invalid form.
    /// Instruction encoded in an invalid form, such as a load with
    /// update whose RA is 0 or RT. Payload is the mnemonic.
    #[error("PPU invalid instruction form: {0}")]
    InvalidForm(&'static str),
}

impl PpuFault {
    /// Returns the guest fault code published by the execution unit.
    pub fn guest_code(&self) -> u32 {
        match *self {
            Self::PcOutOfRange(_) => crate::FAULT_PC_OUT_OF_RANGE,
            Self::InvalidAddress(_) => crate::FAULT_INVALID_ADDRESS,
            Self::UnsupportedSyscall(number) => {
                crate::FAULT_UNSUPPORTED_SYSCALL | (number as u32 & 0xffff)
            }
            Self::UnimplementedInstruction(opcode) => {
                crate::FAULT_UNIMPLEMENTED_INSN | (opcode as u32 & 0xffff)
            }
            Self::ProgramTrap(to) => crate::FAULT_PROGRAM_TRAP | (u32::from(to) & 0xffff),
            Self::AlignmentInterrupt(_) => crate::FAULT_ALIGNMENT_INTERRUPT,
            Self::InvalidForm(_) => crate::FAULT_INVALID_FORM,
        }
    }
}
