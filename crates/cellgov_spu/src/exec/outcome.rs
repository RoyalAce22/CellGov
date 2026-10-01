//! The outcome of one SPU step and the faults it can raise.

use cellgov_effects::Effect;
use cellgov_exec::YieldReason;

use crate::stop::SpuStopKind;

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
    /// The instruction stopped the SPU. The caller records the stop in
    /// [`crate::state::SpuState::stop`] and moves the PC to the resume
    /// address.
    Stop {
        /// What stopped the SPU.
        kind: SpuStopKind,
        /// The instruction's 14-bit signal field, or zero.
        signal: u16,
    },
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
    /// A defined MFC command the SPU queue accepts and this model does
    /// not run. The model runs every such command, so no defined opcode
    /// reaches it.
    ///
    /// The variant carries the whole 32-bit word, class ids included.
    #[error("SPU unsupported MFC command word 0x{0:08x}")]
    UnsupportedMfcCommand(u32),
    /// A channel whose capacity the model does not know.
    #[error("SPU unsupported channel rchcnt 0x{0:02x}")]
    UnsupportedChannelCount(u8),
    /// An access to a decrementer channel. The console's decrementer
    /// counts down at the time-base frequency, so its value tracks
    /// elapsed hardware time; CellGov's SPU time does not yet follow the
    /// hardware's cycles, so no value it could return would be the
    /// console's, and the model refuses the access by name.
    #[error(
        "SPU decrementer channel {} 0x{channel:02x}: the decrementer is not modeled, \
         since CellGov's SPU time is not the console's cycle count",
        if *is_count { "rchcnt" } else if *channel == cellgov_ps3_abi::hw::spu::SPU_WR_DEC { "wrch" } else { "rdch" }
    )]
    DecrementerUnmodeled {
        /// `SPU_WrDec` or `SPU_RdDec`.
        channel: u8,
        /// Whether the access was `rchcnt`.
        is_count: bool,
    },
    /// A channel access whose stall no event can end: a tag-status
    /// read with no update request, which only an interrupt could end.
    #[error("SPU channel 0x{0:02x} access stalls")]
    ChannelStall(u8),
    /// A tag-status update request with a reserved value.
    ///
    /// The word is the value the guest wrote.
    #[error("SPU reserved tag-status update request 0x{0:08x}")]
    ReservedTagUpdate(u32),
    /// A conversion whose I8 field gives a scale outside 0..=127, where
    /// the result is undefined.
    ///
    /// The byte is the I8 field.
    #[error("SPU conversion I8 {0} gives an undefined scale")]
    UndefinedConversionScale(u8),
    /// A taken indirect branch with both the D and E feature bits set,
    /// whose effect on the interrupt-enable state is undefined.
    ///
    /// The word is the branch's address.
    #[error("SPU indirect branch at 0x{0:05x} sets both D and E")]
    UndefinedInterruptControl(u32),
}
