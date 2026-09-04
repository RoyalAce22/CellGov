//! Binary trace writer: append-only `Vec<u8>` of encoded [`TraceRecord`]s.
//!
//! Records are appended in emission order; replay relies on this. The writer
//! buffers in memory with no file I/O and no flushing policy.

use crate::level::TraceLevel;
use crate::record::TraceRecord;

/// Binary trace writer.
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

    /// Append `record` if its level passes the filter; returns whether it was written.
    ///
    /// Refuses [`TraceRecord::RunIdentity`], which enters the stream only
    /// through [`Self::record_header`].
    pub fn record(&mut self, record: &TraceRecord) -> bool {
        if matches!(record, TraceRecord::RunIdentity { .. }) {
            return false;
        }
        let level_bit = 1u8 << (record.level() as u8);
        if self.enabled_mask & level_bit == 0 {
            return false;
        }
        record.encode(&mut self.buf);
        self.record_count += 1;
        true
    }

    /// Append the stream's [`TraceRecord::RunIdentity`] header whatever
    /// the level filter says; returns whether it was written.
    ///
    /// Returns `false` and writes nothing when:
    /// - `record` is not [`TraceRecord::RunIdentity`];
    /// - the writer already holds bytes.
    pub fn record_header(&mut self, record: &TraceRecord) -> bool {
        if !matches!(record, TraceRecord::RunIdentity { .. }) || !self.buf.is_empty() {
            return false;
        }
        record.encode(&mut self.buf);
        self.record_count += 1;
        true
    }

    /// Records actually written (post-filter).
    #[inline]
    pub fn record_count(&self) -> usize {
        self.record_count
    }

    /// Bytes in the trace buffer.
    #[inline]
    pub fn byte_len(&self) -> usize {
        self.buf.len()
    }

    /// Borrow the trace bytes.
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

    /// Drop accumulated records, preserving allocator capacity and
    /// level-filter mask (unlike [`Self::take_bytes`]).
    #[inline]
    pub fn clear(&mut self) {
        self.buf.clear();
        self.record_count = 0;
    }
}

#[cfg(test)]
#[path = "tests/writer_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/writer_header_tests.rs"]
mod header_tests;
