//! [`PpuFault`] -- PPU-specific architectural fault categories.

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
}
