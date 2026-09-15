//! The store watch: every write the runtime lands in one guest window.
//!
//! `CELLGOV_STORE_WATCH=<addr>:<len>` names the window and
//! `CELLGOV_STORE_WATCH_PATH=<path>` names the capture. The watch
//! records each write that overlaps `[addr, addr+len)`. The capture
//! uses the binary format of the patch set's `cellgov_store_watch.h`
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

use super::error::TapError;
use super::parse::hex_pair;
use super::record_file::RecordFile;

const SPEC_VAR: &str = "CELLGOV_STORE_WATCH";
const PATH_VAR: &str = "CELLGOV_STORE_WATCH_PATH";

/// The largest window the watch accepts.
const MAX_LEN: u64 = 0x10000;

/// One past the last address a u32 header and record field can name.
const WINDOW_END: u64 = 1 << 32;

/// One record's size.
pub(super) const RECORD_LEN: usize = 28;

/// What the two env vars ask the watch for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StoreWatchSpec {
    /// First guest address of the window; the window ends at or below
    /// 4 GiB.
    pub addr: u64,
    /// Window length in bytes, `1..=0x10000`.
    pub len: u64,
    /// Where the capture goes.
    pub path: PathBuf,
}

impl StoreWatchSpec {
    /// Read the spec from the two variables' values; `None` when
    /// neither is set. An empty value reads as unset.
    ///
    /// # Errors
    ///
    /// Returns a [`TapError`] when:
    ///
    /// - the window does not parse or is out of range
    /// - one variable is set without the other
    pub(super) fn parse(spec: Option<&str>, path: Option<&str>) -> Result<Option<Self>, TapError> {
        let spec = spec.unwrap_or_default().trim();
        let path = path.unwrap_or_default();
        match (spec.is_empty(), path.is_empty()) {
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
        let (addr, len) = hex_pair(SPEC_VAR, spec, "<addr>:<len>")?;
        if len == 0 || len > MAX_LEN {
            return Err(TapError::OutOfRange {
                var: SPEC_VAR,
                value: len,
                range: "1..=0x10000",
            });
        }
        // The header's address and each record's `ea` are u32, as in
        // the patch set's hook. For a window past 4 GiB, the header
        // names one address and the watch covers another.
        if addr.saturating_add(len) > WINDOW_END {
            return Err(TapError::OutOfRange {
                var: SPEC_VAR,
                value: addr,
                range: "a window ending at or below 0x1_0000_0000",
            });
        }
        Ok(Some(Self {
            addr,
            len,
            path: PathBuf::from(path),
        }))
    }

    /// The whole file header: magic, version 1, window address and
    /// length.
    pub(super) fn header(&self) -> [u8; 16] {
        let mut header = [0u8; 16];
        header[0..4].copy_from_slice(b"CGSW");
        header[4..8].copy_from_slice(&1u32.to_le_bytes());
        header[8..12].copy_from_slice(&(self.addr as u32).to_le_bytes());
        header[12..16].copy_from_slice(&(self.len as u32).to_le_bytes());
        header
    }
}

/// One record: `{ record u64, pc u32, ea u32, width u32, value u64 }`, little-endian.
pub(super) fn pack_record(record: u64, pc: u32, ea: u64, width: u32, value: u64) -> [u8; 28] {
    let mut out = [0u8; RECORD_LEN];
    out[0..8].copy_from_slice(&record.to_le_bytes());
    out[8..12].copy_from_slice(&pc.to_le_bytes());
    out[12..16].copy_from_slice(&(ea as u32).to_le_bytes());
    out[16..20].copy_from_slice(&width.to_le_bytes());
    out[20..28].copy_from_slice(&value.to_le_bytes());
    out
}

/// The watch's window, record counter and capture.
pub(super) struct StoreWatch<W: Write> {
    addr: u64,
    len: u64,
    records: u64,
    out: RecordFile<W>,
}

impl<W: Write> StoreWatch<W> {
    /// A watch on `spec`'s window writing to `out`, whose header is
    /// already written.
    pub(super) fn new(spec: &StoreWatchSpec, out: RecordFile<W>) -> Self {
        Self {
            addr: spec.addr,
            len: spec.len,
            records: 0,
            out,
        }
    }

    /// Record the write of `bytes` at `ea` when it overlaps the window.
    ///
    /// `pc` is the last PPU instruction dispatched. The record names the
    /// whole write, and a reader intersects it with the window.
    pub(super) fn write(&mut self, pc: u32, ea: u64, bytes: &[u8]) {
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
        self.out.append(&pack_record(
            record,
            pc,
            ea,
            bytes.len() as u32,
            u64::from_le_bytes(value),
        ));
    }

    /// The writer the records went to.
    #[cfg(test)]
    pub(super) fn into_inner(self) -> W {
        self.out.into_inner()
    }
}

#[cfg(test)]
#[path = "tests/store_watch_tests.rs"]
mod tests;
