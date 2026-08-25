//! Position-carrying wrapper for a trace-stream decode failure, shared
//! by every reader in this crate that walks a binary trace end to end.

use cellgov_trace::DecodeError;

/// A record in a run's own trace stream failed to decode.
///
/// The stream is this workspace's encoder's output, so a bad record
/// is a host invariant break: the records before it are a prefix, and
/// a comparison over that prefix could mask or fabricate a divergence
/// at the cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("trace record {index} at byte offset {offset} failed to decode: {source}")]
pub struct TraceDecodeError {
    /// Zero-based index of the record that failed, counting every
    /// record kind in the stream.
    pub index: usize,
    /// Byte offset of that record's first byte in the trace stream.
    pub offset: usize,
    /// The decoder's own reason.
    #[source]
    pub source: DecodeError,
}
