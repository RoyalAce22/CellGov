//! Binary trace writer.
//!
//! `TraceWriter` is a thin wrapper around a `Vec<u8>` that appends
//! [`TraceRecord`] encodings in the order they were emitted. The trace
//! format is binary from day one; this writer produces binary directly,
//! so the buffer it accumulates is the same shape any future on-disk
//! format will use.
//!
//! Currently the writer is in-memory: a Vec, no file I/O, no flushing
//! policy. The runtime calls `record(...)` for each event; tests and
//! the testkit runner pull the bytes out at the end and feed them to
//! [`crate::reader::TraceReader`] for assertions. Persisting to disk is
//! a separate slice that swaps the backing buffer for a `Write`-trait
//! sink without changing the public surface here.
//!
//! ## Filtering
//!
//! Each record carries a [`TraceLevel`] via [`TraceRecord::level`].
//! The writer supports an optional filter that drops records whose
//! level is not enabled, so that high-volume categories can be
//! filtered without reworking the writer later. The filter is set at construction time and applied
//! per call to [`TraceWriter::record`].

use crate::level::TraceLevel;
use crate::record::TraceRecord;

/// A binary trace writer.
///
/// Owns the buffer of encoded records. Records are appended in
/// emission order; the runtime relies on this for replay.
#[derive(Debug, Clone, Default)]
pub struct TraceWriter {
    buf: Vec<u8>,
    enabled_mask: u8,
    record_count: usize,
}

impl TraceWriter {
    /// Construct a writer that records every level.
    #[inline]
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            enabled_mask: 0xff,
            record_count: 0,
        }
    }

    /// Construct a writer that only records the given levels.
    ///
    /// A record whose [`TraceRecord::level`] is not in `levels` is
    /// dropped at the boundary -- it never reaches the buffer and
    /// does not increment [`TraceWriter::record_count`].
    pub fn with_levels(levels: &[TraceLevel]) -> Self {
        let mut mask = 0u8;
        for l in levels {
            mask |= 1 << (*l as u8);
        }
        Self {
            buf: Vec::new(),
            enabled_mask: mask,
            record_count: 0,
        }
    }

    /// Append `record` to the trace, if its level passes the filter.
    /// Returns `true` if the record was actually written, `false` if
    /// it was dropped by the level filter.
    pub fn record(&mut self, record: &TraceRecord) -> bool {
        let level_bit = 1u8 << (record.level() as u8);
        if self.enabled_mask & level_bit == 0 {
            return false;
        }
        record.encode(&mut self.buf);
        self.record_count += 1;
        true
    }

    /// Number of records actually written (post-filter).
    #[inline]
    pub fn record_count(&self) -> usize {
        self.record_count
    }

    /// Total number of bytes in the trace buffer.
    #[inline]
    pub fn byte_len(&self) -> usize {
        self.buf.len()
    }

    /// Borrow the trace bytes. Suitable for feeding into
    /// [`crate::reader::TraceReader`] or for serialization.
    #[inline]
    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }

    /// Take ownership of the trace bytes, clearing the writer.
    #[inline]
    pub fn take_bytes(&mut self) -> Vec<u8> {
        self.record_count = 0;
        std::mem::take(&mut self.buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::StateHash;
    use crate::record::{HashCheckpointKind, TracedYieldReason};
    use cellgov_event::UnitId;
    use cellgov_time::{Budget, Epoch, GuestTicks};

    fn scheduled() -> TraceRecord {
        TraceRecord::UnitScheduled {
            unit: UnitId::new(0),
            granted_budget: Budget::new(1),
            time: GuestTicks::ZERO,
            epoch: Epoch::ZERO,
        }
    }

    fn commit() -> TraceRecord {
        TraceRecord::CommitApplied {
            unit: UnitId::new(0),
            writes_committed: 1,
            effects_deferred: 0,
            fault_discarded: false,
            epoch_after: Epoch::new(1),
        }
    }

    fn hash_checkpoint() -> TraceRecord {
        TraceRecord::StateHashCheckpoint {
            kind: HashCheckpointKind::CommittedMemory,
            hash: StateHash::new(42),
        }
    }

    fn step() -> TraceRecord {
        TraceRecord::StepCompleted {
            unit: UnitId::new(0),
            yield_reason: TracedYieldReason::BudgetExhausted,
            consumed_budget: Budget::new(1),
            time_after: GuestTicks::new(1),
        }
    }

    #[test]
    fn empty_writer_is_zero_records_zero_bytes() {
        let w = TraceWriter::new();
        assert_eq!(w.record_count(), 0);
        assert_eq!(w.byte_len(), 0);
        assert!(w.bytes().is_empty());
    }

    #[test]
    fn writes_increment_count_and_buffer() {
        let mut w = TraceWriter::new();
        assert!(w.record(&scheduled()));
        assert!(w.record(&step()));
        assert!(w.record(&commit()));
        assert_eq!(w.record_count(), 3);
        // 33 + 26 + 26 = 85 bytes per the format documentation.
        assert_eq!(w.byte_len(), 85);
    }

    #[test]
    fn level_filter_drops_disabled_records() {
        // Only record commits and hashes; scheduling drops out.
        let mut w = TraceWriter::with_levels(&[TraceLevel::Commits, TraceLevel::Hashes]);
        assert!(!w.record(&scheduled()));
        assert!(!w.record(&step()));
        assert!(w.record(&commit()));
        assert!(w.record(&hash_checkpoint()));
        assert_eq!(w.record_count(), 2);
    }

    #[test]
    fn empty_filter_drops_everything() {
        let mut w = TraceWriter::with_levels(&[]);
        assert!(!w.record(&scheduled()));
        assert!(!w.record(&commit()));
        assert_eq!(w.record_count(), 0);
        assert_eq!(w.byte_len(), 0);
    }

    #[test]
    fn take_bytes_clears_writer() {
        let mut w = TraceWriter::new();
        w.record(&scheduled());
        let b = w.take_bytes();
        assert!(!b.is_empty());
        assert_eq!(w.byte_len(), 0);
        assert_eq!(w.record_count(), 0);
    }
}
