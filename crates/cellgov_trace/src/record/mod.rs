//! Structured trace record types and their binary encoding.
//!
//! Each record is a 1-byte tag followed by a fixed-length variant
//! payload, all multi-byte integers little-endian; there is no length
//! field after the tag. Tags, per-variant field layouts, and the
//! discriminants of every `Traced*` mirror enum and
//! `HashCheckpointKind` are part of the binary trace contract; new
//! record variants append with strictly greater tags.

mod codec;
mod error;
mod reasons;
mod trace_record;

pub use error::DecodeError;
pub use reasons::{
    HashCheckpointKind, HostWriter, TracedBlockReason, TracedEffectKind,
    TracedInvariantBreakReason, TracedSyscallDisposition, TracedWakeReason, TracedYieldReason,
};
pub use trace_record::{TraceRecord, TRACE_FORMAT_VERSION};

#[cfg(test)]
#[path = "tests/record_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/record_len_tests.rs"]
mod len_tests;

#[cfg(test)]
#[path = "tests/record_identity_tests.rs"]
mod identity_tests;

#[cfg(test)]
#[path = "tests/record_host_write_tests.rs"]
mod host_write_tests;

#[cfg(test)]
#[path = "tests/record_format_version_tests.rs"]
mod format_version_tests;

#[cfg(test)]
#[path = "tests/record_scheme_tests.rs"]
mod scheme_tests;
