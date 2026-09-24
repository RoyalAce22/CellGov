//! The outcome of one SPU step and the faults it can raise.

use cellgov_effects::Effect;
use cellgov_exec::YieldReason;

/// Outcome of executing a single SPU instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpuStepOutcome {
    /// Advance PC by 4 and continue.
    Continue,
    /// PC was set by the instruction; do not advance.
    Branch,
    /// Yield to runtime with Effects.
    Yield {
        /// Effects to commit.
        effects: Vec<Effect>,
        /// Why the unit is yielding.
        reason: YieldReason,
    },
    /// Memory read the caller must service from the committed snapshot.
    ///
    /// The caller copies `size` bytes from `ea` into LS at `lsa`. When
    /// `acquire_line` is set (MFC_GETLLAR), the caller then:
    /// - installs the reservation,
    /// - sets the atomic status to `MFC_ATOMIC_STAT_G`,
    /// - emits an `Effect::ReservationAcquire` for that line.
    MemoryRead {
        /// Guest effective address to read from. For a line read this
        /// is the line's own address, whatever the guest wrote.
        ea: u64,
        /// Local store destination address.
        lsa: u32,
        /// Number of bytes to read.
        size: u32,
        /// Canonical line address to install a reservation for, or `None`.
        acquire_line: Option<u64>,
    },
    /// Instruction caused an architecture fault.
    Fault(SpuFault),
}

/// SPU-specific fault categories.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpuFault {
    /// LS access outside valid range.
    #[error("SPU LS access out of range at 0x{0:08x}")]
    LsOutOfRange(u32),
    /// Unsupported channel operation.
    #[error("SPU unsupported channel {} 0x{channel:02x}", if *is_write { "wrch" } else { "rdch" })]
    UnsupportedChannel {
        /// Channel number.
        channel: u8,
        /// Whether it was a read or write.
        is_write: bool,
    },
    /// An MFC command word this model will not enqueue.
    ///
    /// The variant carries the whole 32-bit word. A reader can then tell
    /// a refusal the reserved bit raised from one the opcode raised.
    #[error("SPU unsupported MFC command word 0x{0:08x}")]
    UnsupportedMfcCommand(u32),
    /// A channel whose capacity the model does not know.
    #[error("SPU unsupported channel rchcnt 0x{0:02x}")]
    UnsupportedChannelCount(u8),
    /// An MFC command whose staged tag id is outside the architected
    /// range.
    ///
    /// The completion path publishes `1 << tag_id` into a 32-bit
    /// tag-status word, so a value past the range has no bit to set.
    #[error("SPU MFC command tag id {0} is outside 0..31")]
    TagIdOutOfRange(u32),
}
