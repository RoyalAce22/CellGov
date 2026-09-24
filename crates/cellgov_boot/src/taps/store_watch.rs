//! The store watch: every write the runtime lands in one guest window.
//!
//! The watch records each write that overlaps `[addr, addr+len)`. The
//! capture uses the binary format of the patch set's `cellgov_store_watch.h`
//! hook, so one reader consumes both logs.
//!
//! The runtime reports each write as it lands. A record holds these
//! fields:
//!
//! - `record`: the index of the record in the capture, from 0.
//! - `pc`: the last PPU instruction dispatched before the report. It
//!   names the active code region, and can differ from the instruction
//!   that stored.
//! - `ea`: the address of the first byte written.
//! - `width`: the length of the write in bytes.
//! - `value`: the first 8 bytes of the write, zero-padded for a
//!   shorter write.

use std::io::Write;
use std::path::PathBuf;

use super::record_file::{FirstFailure, RecordFile};

/// The largest window the watch accepts.
pub const MAX_LEN: u64 = 0x10000;

/// One past the last address a u32 header and record field can name.
/// The header's address and each record's `ea` are u32, as in the patch
/// set's hook, so a window past 4 GiB would name one address and cover
/// another.
pub const WINDOW_END: u64 = 1 << 32;

/// One record's size.
pub const RECORD_LEN: usize = 28;

/// The window the watch covers and where it records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreWatchSpec {
    /// First guest address of the window; the window ends at or below
    /// 4 GiB.
    pub addr: u64,
    /// Window length in bytes, `1..=0x10000`.
    pub len: u64,
    /// Where the capture goes.
    pub path: PathBuf,
}

impl StoreWatchSpec {
    /// The whole file header: magic, version 1, window address and
    /// length.
    #[must_use]
    pub fn header(&self) -> [u8; 16] {
        let mut header = [0u8; 16];
        header[0..4].copy_from_slice(b"CGSW");
        header[4..8].copy_from_slice(&1u32.to_le_bytes());
        header[8..12].copy_from_slice(&(self.addr as u32).to_le_bytes());
        header[12..16].copy_from_slice(&(self.len as u32).to_le_bytes());
        header
    }
}

/// One record: `{ record u64, pc u32, ea u32, width u32, value u64 }`, little-endian.
#[must_use]
pub fn pack_record(record: u64, pc: u32, ea: u64, width: u32, value: u64) -> [u8; 28] {
    let mut out = [0u8; RECORD_LEN];
    out[0..8].copy_from_slice(&record.to_le_bytes());
    out[8..12].copy_from_slice(&pc.to_le_bytes());
    out[12..16].copy_from_slice(&(ea as u32).to_le_bytes());
    out[16..20].copy_from_slice(&width.to_le_bytes());
    out[20..28].copy_from_slice(&value.to_le_bytes());
    out
}

/// The watch's window, record counter and capture.
pub struct StoreWatch<W: Write> {
    addr: u64,
    len: u64,
    records: u64,
    out: RecordFile<W>,
    failure: FirstFailure,
}

impl<W: Write> StoreWatch<W> {
    /// A watch on `spec`'s window writing to `out`, whose header is
    /// already written.
    pub fn new(spec: &StoreWatchSpec, out: RecordFile<W>) -> Self {
        Self {
            addr: spec.addr,
            len: spec.len,
            records: 0,
            out,
            failure: FirstFailure::default(),
        }
    }

    /// Record the write of `bytes` at `ea` when it overlaps the window.
    ///
    /// `pc` is the last PPU instruction dispatched. The record names the
    /// whole write, and a reader intersects it with the window.
    pub fn write(&mut self, pc: u32, ea: u64, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let end = ea.saturating_add(bytes.len() as u64);
        if end <= self.addr || ea >= self.addr.saturating_add(self.len) {
            return;
        }
        let mut value = [0u8; 8];
        let take = bytes.len().min(8);
        value[..take].copy_from_slice(&bytes[..take]);
        let record = self.records;
        self.records = self.records.wrapping_add(1);
        self.failure.note(self.out.append(&pack_record(
            record,
            pc,
            ea,
            bytes.len() as u32,
            u64::from_le_bytes(value),
        )));
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
#[path = "tests/store_watch_tests.rs"]
mod tests;
