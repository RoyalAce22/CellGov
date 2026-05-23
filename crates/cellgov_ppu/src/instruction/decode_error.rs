//! [`PpuDecodeError`] -- raised by the decoder when no
//! [`super::PpuInstruction`] variant matches a 32-bit word.

/// Why decoding failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PpuDecodeError {
    /// No matching encoding for this 32-bit word.
    #[error("unsupported PPU instruction 0x{0:08x}")]
    Unsupported(u32),
}
