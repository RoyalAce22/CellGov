//! cellgov_trace -- structured trace events, deterministic replay format,
//! state hash checkpoints.
//!
//! Trace format is binary, not text. Trace levels (scheduling,
//! effects, commits, hashes) let high-volume categories be filtered without
//! reworking the writer. Text rendering is a downstream tool over the binary
//! format, never the source of truth.
//!
//! Records every scheduling decision, every effect, every commit, every
//! wake/block transition, every guest-time advance. State hashing happens at
//! controlled checkpoints so deterministic replay can compare committed memory,
//! runnable queues, sync state, and unit status.

pub mod hash;
pub mod level;
pub mod reader;
pub mod record;
pub mod writer;

pub use hash::StateHash;
pub use level::TraceLevel;
pub use reader::TraceReader;
pub use record::{
    DecodeError, HashCheckpointKind, TraceRecord, TracedBlockReason, TracedEffectKind,
    TracedWakeReason, TracedYieldReason,
};
pub use writer::TraceWriter;
