//! Iterator decoding a byte slice produced by [`crate::writer::TraceWriter`].
//!
//! On decode failure the iterator yields `Some(Err(...))` once and then `None`
//! forever -- no recovery, no skip-ahead.

use crate::record::{DecodeError, TraceRecord};

/// Iterator over records encoded in a trace byte buffer.
pub struct TraceReader<'a> {
    bytes: &'a [u8],
    pos: usize,
    failed: bool,
}

impl<'a> TraceReader<'a> {
    /// Construct a reader over `bytes`.
    #[inline]
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            pos: 0,
            failed: false,
        }
    }

    /// Bytes consumed so far.
    #[inline]
    pub fn position(&self) -> usize {
        self.pos
    }
}

impl Iterator for TraceReader<'_> {
    type Item = Result<TraceRecord, DecodeError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        if self.pos >= self.bytes.len() {
            return None;
        }
        match TraceRecord::decode(&self.bytes[self.pos..]) {
            Ok((record, n)) => {
                self.pos += n;
                Some(Ok(record))
            }
            Err(e) => {
                self.failed = true;
                Some(Err(e))
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/reader_tests.rs"]
mod tests;
