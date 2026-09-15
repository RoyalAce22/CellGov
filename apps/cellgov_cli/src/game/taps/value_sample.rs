//! The value sample: one guest range read at step boundaries.
//!
//! The sample reads the range's current bytes whichever path wrote
//! them; the store watch answers which writes landed there.
//!
//! Env vars:
//!
//!   CELLGOV_VALUE_SAMPLE         `ADDR:WIDTH` in hex (e.g.
//!                                `0x91FE9C:4`); width in `[1, 256]`.
//!   CELLGOV_VALUE_SAMPLE_PATH    Output file path.
//!   CELLGOV_VALUE_SAMPLE_STRIDE  Optional decimal stride; default 1
//!                                (every step). Zero is out of range.
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

use super::error::TapError;
use super::parse::hex_pair;
use super::record_file::RecordFile;

const SPEC_VAR: &str = "CELLGOV_VALUE_SAMPLE";
const PATH_VAR: &str = "CELLGOV_VALUE_SAMPLE_PATH";
const STRIDE_VAR: &str = "CELLGOV_VALUE_SAMPLE_STRIDE";

/// What the three env vars ask the sample for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ValueSampleSpec {
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
    /// Read the spec from the three variables' values; `None` when
    /// none of them is set. An empty value reads as unset.
    ///
    /// # Errors
    ///
    /// Returns a [`TapError`] when:
    ///
    /// - the range or the stride does not parse or is out of range
    /// - the range is set without the path, or the path without the range
    /// - the stride is set without the range and the path
    pub(super) fn parse(
        spec: Option<&str>,
        path: Option<&str>,
        stride: Option<&str>,
    ) -> Result<Option<Self>, TapError> {
        let spec = spec.unwrap_or_default().trim();
        let path = path.unwrap_or_default();
        let stride = stride.map(str::trim).filter(|s| !s.is_empty());
        match (spec.is_empty(), path.is_empty()) {
            (true, true) if stride.is_some() => {
                return Err(TapError::Unpaired {
                    set: STRIDE_VAR,
                    missing: SPEC_VAR,
                })
            }
            (true, true) => return Ok(None),
            (false, true) => {
                return Err(TapError::Unpaired {
                    set: SPEC_VAR,
                    missing: PATH_VAR,
                })
            }
            (true, false) => {
                return Err(TapError::Unpaired {
                    set: PATH_VAR,
                    missing: SPEC_VAR,
                })
            }
            (false, false) => {}
        }
        let (addr, width) = hex_pair(SPEC_VAR, spec, "<addr>:<width>")?;
        if width == 0 || width > 256 {
            return Err(TapError::OutOfRange {
                var: SPEC_VAR,
                value: width,
                range: "1..=0x100",
            });
        }
        // The header's address field is u32. For a wider address, the
        // header names one address and the sample reads another.
        if addr > u64::from(u32::MAX) {
            return Err(TapError::OutOfRange {
                var: SPEC_VAR,
                value: addr,
                range: "32 bits",
            });
        }
        let stride = match stride {
            None => 1,
            Some(s) => s.parse::<u64>().map_err(|source| TapError::BadNumber {
                var: STRIDE_VAR,
                token: s.to_string(),
                source,
            })?,
        };
        if stride == 0 {
            return Err(TapError::OutOfRange {
                var: STRIDE_VAR,
                value: 0,
                range: "1..",
            });
        }
        Ok(Some(Self {
            addr,
            width: width as u32,
            stride,
            path: PathBuf::from(path),
        }))
    }

    /// The whole file header: magic, version 2, address and width.
    pub(super) fn header(&self) -> [u8; 16] {
        let mut header = [0u8; 16];
        header[0..4].copy_from_slice(b"CGVS");
        header[4..8].copy_from_slice(&2u32.to_le_bytes());
        header[8..12].copy_from_slice(&(self.addr as u32).to_le_bytes());
        header[12..16].copy_from_slice(&self.width.to_le_bytes());
        header
    }
}

/// One record: `{ step u64, status u8, actual_len u32, value[width] }`, little-endian.
pub(super) fn pack_record(step: u64, status: u8, actual_len: u32, value: &[u8]) -> Vec<u8> {
    let mut record = Vec::with_capacity(13 + value.len());
    record.extend_from_slice(&step.to_le_bytes());
    record.push(status);
    record.extend_from_slice(&actual_len.to_le_bytes());
    record.extend_from_slice(value);
    record
}

/// The sample's range, stride and capture.
pub(super) struct ValueSample<W: Write> {
    addr: u64,
    width: u32,
    stride: u64,
    out: RecordFile<W>,
}

impl<W: Write> ValueSample<W> {
    /// A sample of `spec`'s range writing to `out`, whose header is
    /// already written.
    pub(super) fn new(spec: &ValueSampleSpec, out: RecordFile<W>) -> Self {
        Self {
            addr: spec.addr,
            width: spec.width,
            stride: spec.stride,
            out,
        }
    }

    /// Record the range when `step` is a stride boundary.
    ///
    /// `memory` holds the range before the batch of step `step` commits;
    /// see [`cellgov_core::RuntimeTap::step`].
    pub(super) fn step(&mut self, step: u64, memory: &GuestMemory) {
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
        self.out
            .append(&pack_record(step, status, actual_len, &value));
    }

    /// The writer the records went to.
    #[cfg(test)]
    pub(super) fn into_inner(self) -> W {
        self.out.into_inner()
    }
}

#[cfg(test)]
#[path = "tests/value_sample_tests.rs"]
mod tests;
