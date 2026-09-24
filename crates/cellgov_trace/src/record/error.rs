//! Why decoding a trace record stream failed.

use super::trace_record::TRACE_FORMAT_VERSION;

/// Why decoding a trace record stream failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// Byte stream ended mid-record.
    #[error("byte stream ended mid-record")]
    Truncated,
    /// Record tag byte is not a known variant.
    #[error("unknown record tag 0x{0:02x}")]
    UnknownTag(u8),
    /// Yield-reason byte is not a known variant.
    #[error("unknown yield reason 0x{0:02x}")]
    UnknownYieldReason(u8),
    /// Hash-checkpoint-kind byte is not a known variant.
    #[error("unknown hash-checkpoint kind 0x{0:02x}")]
    UnknownHashKind(u8),
    /// A flag byte (`fault_discarded`, `PpuStateFull` reservation tag)
    /// was neither 0 nor 1.
    #[error("flag byte is neither 0 nor 1: 0x{0:02x}")]
    InvalidBool(u8),
    /// Effect-kind byte is not a known variant.
    #[error("unknown effect kind 0x{0:02x}")]
    UnknownEffectKind(u8),
    /// Block-reason byte is not a known variant.
    #[error("unknown block reason 0x{0:02x}")]
    UnknownBlockReason(u8),
    /// Wake-reason byte is not a known variant.
    #[error("unknown wake reason 0x{0:02x}")]
    UnknownWakeReason(u8),
    /// Invariant-break-reason byte is not a known variant.
    #[error("unknown invariant break reason 0x{0:02x}")]
    UnknownInvariantBreakReason(u8),
    /// Syscall-disposition byte is not a known variant.
    #[error("unknown syscall disposition 0x{0:02x}")]
    UnknownSyscallDisposition(u8),
    /// Host-writer byte is not a known variant.
    #[error("unknown host writer 0x{0:02x}")]
    UnknownHostWriter(u8),
    /// The header names a trace format other than [`TRACE_FORMAT_VERSION`].
    ///
    /// Each format fixes its own header width, so the decoder cannot
    /// find where the record after that header starts.
    #[error("trace format {0}, this build reads format {v}", v = TRACE_FORMAT_VERSION)]
    UnsupportedFormatVersion(u32),
}

impl DecodeError {
    pub(super) fn unknown_yield_reason(v: u8) -> Self {
        Self::UnknownYieldReason(v)
    }

    pub(super) fn unknown_hash_kind(v: u8) -> Self {
        Self::UnknownHashKind(v)
    }

    pub(super) fn unknown_block_reason(v: u8) -> Self {
        Self::UnknownBlockReason(v)
    }

    pub(super) fn unknown_wake_reason(v: u8) -> Self {
        Self::UnknownWakeReason(v)
    }

    pub(super) fn unknown_effect_kind(v: u8) -> Self {
        Self::UnknownEffectKind(v)
    }

    pub(super) fn unknown_invariant_break_reason(v: u8) -> Self {
        Self::UnknownInvariantBreakReason(v)
    }

    pub(super) fn unknown_syscall_disposition(v: u8) -> Self {
        Self::UnknownSyscallDisposition(v)
    }

    pub(super) fn unknown_host_writer(v: u8) -> Self {
        Self::UnknownHostWriter(v)
    }
}
