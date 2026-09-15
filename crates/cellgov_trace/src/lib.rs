//! Structured trace records, binary serialization, and state-hash checkpoints.
//!
//! Text rendering is a downstream tool over the binary format, never the source
//! of truth.

#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(
    not(test),
    forbid(
        clippy::disallowed_methods,
        clippy::disallowed_macros,
        clippy::print_stdout,
        clippy::print_stderr,
        clippy::dbg_macro
    )
)]

pub mod hash;
pub mod level;
pub mod reader;
pub mod record;
pub mod writer;

pub use hash::StateHash;
pub use level::TraceLevel;
pub use reader::TraceReader;
pub use record::{
    DecodeError, HashCheckpointKind, HostWriter, TraceRecord, TracedBlockReason, TracedEffectKind,
    TracedInvariantBreakReason, TracedSyscallDisposition, TracedWakeReason, TracedYieldReason,
    TRACE_FORMAT_VERSION,
};
pub use writer::TraceWriter;
