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
    // [PPC-Book2 p:25 s:3.3] stwcx./stdcx.: same alignment contract; the architecture permits "alignment error handler" OR "boundedly undefined", RPCS3 chose the handler-throw path (PPUThread.cpp:3078/3224).
    /// Reservation operand (`lwarx` / `ldarx` / `stwcx.` / `stdcx.`)
    /// EA not aligned to the operand size (4 or 8 bytes). Payload
    /// is the misaligned EA.
    #[error("PPU alignment interrupt on misaligned reservation EA 0x{0:016x}")]
    AlignmentInterrupt(u64),
}
