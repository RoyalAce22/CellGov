//! The value sample: one guest range read at step boundaries.
//!
//! The sample reads the range's current bytes whichever path wrote
//! them; the store watch answers which writes landed there. The range
//! is `1..=`[`MAX_WIDTH`] bytes at an address that fits 32 bits, read
//! every `stride`-th step.
//!
//! File format (little-endian, no padding): a "CGVS" version-2
//! header (magic, version u32, addr u32, width u32), then one
//! record per sample: step u64, status u8 (0 = unmapped, 1 =
//! full-width read, 2 = short read), actual_len u32, value
//! bytes\[width\] zero-padded. `actual_len` separates a short read's
//! padding from a measured zero.

use std::io::Write;
use std::path::PathBuf;

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

use super::record_file::{FirstFailure, RecordFile};

/// The widest range the sample reads.
pub const MAX_WIDTH: u64 = 256;

/// The range the sample reads, how often, and where it records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueSampleSpec {
    /// First guest address of the range; fits 32 bits.
    pub addr: u64,
    /// Range width in bytes, `1..=256`.
    pub width: u32,
    /// Sample every `stride`-th step; never 0.
    pub stride: u64,
    /// Where the capture goes.
    pub path: PathBuf,
}

impl ValueSampleSpec {
    /// The whole file header: magic, version 2, address and width.
    #[must_use]
    pub fn header(&self) -> [u8; 16] {
        let mut header = [0u8; 16];
        header[0..4].copy_from_slice(b"CGVS");
        header[4..8].copy_from_slice(&2u32.to_le_bytes());
        header[8..12].copy_from_slice(&(self.addr as u32).to_le_bytes());
        header[12..16].copy_from_slice(&self.width.to_le_bytes());
        header
    }
}

/// One record: `{ step u64, status u8, actual_len u32, value[width] }`, little-endian.
#[must_use]
pub fn pack_record(step: u64, status: u8, actual_len: u32, value: &[u8]) -> Vec<u8> {
    let mut record = Vec::with_capacity(13 + value.len());
    record.extend_from_slice(&step.to_le_bytes());
    record.push(status);
    record.extend_from_slice(&actual_len.to_le_bytes());
    record.extend_from_slice(value);
    record
}

/// The sample's range, stride and capture.
pub struct ValueSample<W: Write> {
    addr: u64,
    width: u32,
    stride: u64,
    out: RecordFile<W>,
    failure: FirstFailure,
}

impl<W: Write> ValueSample<W> {
    /// A sample of `spec`'s range writing to `out`, whose header is
    /// already written.
    pub fn new(spec: &ValueSampleSpec, out: RecordFile<W>) -> Self {
        Self {
            addr: spec.addr,
            width: spec.width,
            stride: spec.stride,
            out,
            failure: FirstFailure::default(),
        }
    }

    /// Record the range when `step` is a stride boundary.
    ///
    /// `memory` holds the range before the batch of step `step` commits;
    /// see [`cellgov_core::RuntimeTap::step`].
    pub fn step(&mut self, step: u64, memory: &GuestMemory) {
        if !step.is_multiple_of(self.stride) {
            return;
        }
        let bytes = ByteRange::new(GuestAddr::new(self.addr), u64::from(self.width))
            .and_then(|range| memory.read(range));
        self.record(step, bytes);
    }

    /// Record one sample of `bytes`, the read result; `None` means the range is unmapped.
    ///
    /// A short read keeps its length in `actual_len`, and zeros fill the
    /// rest of the value.
    fn record(&mut self, step: u64, bytes: Option<&[u8]>) {
        let width = self.width as usize;
        let mut value = vec![0u8; width];
        let (status, actual_len) = match bytes {
            Some(b) if b.len() >= width => {
                value.copy_from_slice(&b[..width]);
                (1, width as u32)
            }
            Some(b) => {
                value[..b.len()].copy_from_slice(b);
                (2, b.len() as u32)
            }
            None => (0, 0),
        };
        self.failure.note(
            self.out
                .append(&pack_record(step, status, actual_len, &value)),
        );
    }

    /// The first write failure since the last call, once. The capture
    /// ends at it.
    pub fn take_write_failure(&mut self) -> Option<std::io::Error> {
        self.failure.take()
    }

    /// The writer the records went to.
    #[cfg(test)]
    pub(crate) fn into_inner(self) -> W {
        self.out.into_inner()
    }
}

#[cfg(test)]
#[path = "tests/value_sample_tests.rs"]
mod tests;
