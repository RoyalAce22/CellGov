//! Structured trace records, binary serialization, and state-hash checkpoints.
//!
//! Text rendering is a downstream tool over the binary format, never the source
//! of truth.

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
