//! Reader for the HLE call trace the patched RPCS3 emits via
//! `bridges/rpcs3-patch/0002-cellgov-hle-trace.patch`. The patch's
//! `cellgov_hle_trace.h` header pins the on-disk format.
//!
//! The patch writes one record at every BIND_FUNC entry and exit pair.
//! A record lists the writes the call made to a selected guest region,
//! diffed against the entry-time snapshot. All multi-byte integers are
//! little-endian.
//!
//! The reader streams, so memory stays bounded whatever the trace
//! size. A record whose body does not decode is dropped, and the reader
//! resumes at the next record magic; both come back as
//! [`HleTraceEvent`] values.

use std::collections::BTreeMap;
use std::io::Read;

use thiserror::Error;

/// Header magic written first to an HLE trace file.
pub const HEADER_MAGIC: u32 = 0xC0E6_0001;

/// Per-record magic.
pub const RECORD_MAGIC: u32 = 0xC0E6_0002;

/// Format version this reader understands.
pub const FORMAT_VERSION: u32 = 2;

/// Longest record name the reader accepts, in bytes.
const NAME_LEN_CAP: u32 = 1024;

/// Largest single write payload the reader accepts, in bytes.
const WRITE_SIZE_CAP: u32 = 1 << 20;

/// One HLE call record. Mirrors the binary record on disk.
///
/// `lr` is the PPU LR at HLE entry. For HLE module functions this is
/// the user-code call site (a real PC in the title binary). For
/// syscalls it is the syscall-stub return PC. For synthetic
/// `<guest_code>` drift records it is the LR captured at the prior
/// HLE call's exit (= the user-code site running between the two
/// calls).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HleCallRecord {
    /// Trace step at which the call was recorded.
    pub step: u64,
    /// PPU LR at HLE entry.
    pub lr: u64,
    /// PPU thread id of the caller.
    pub thread_id: u32,
    /// HLE call nesting depth.
    pub depth: u32,
    /// HLE function name.
    pub name: String,
    /// r3..r10 at entry.
    pub args: [u64; 8],
    /// r3 at exit.
    pub ret: u64,
    /// Writes the call made to the selected guest region.
    pub writes: Vec<HleWrite>,
}

impl HleCallRecord {
    /// Whether any write in this record covers a byte of
    /// `[addr, addr + len)`. Both ends saturate at `u64::MAX`.
    pub fn writes_into(&self, addr: u64, len: u64) -> bool {
        let end = addr.saturating_add(len);
        self.writes.iter().any(|w| {
            let w_end = w.addr.saturating_add(w.bytes.len() as u64);
            w.addr < end && addr < w_end
        })
    }
}

/// One guest write recorded inside an HLE call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HleWrite {
    /// Guest address of the first written byte.
    pub addr: u64,
    /// The bytes written.
    pub bytes: Vec<u8>,
}

/// Why the HLE trace could not be read.
#[derive(Debug, Error)]
pub enum HleTraceError {
    /// The underlying reader failed.
    #[error("I/O error: {0}")]
    Io(#[source] std::io::Error),
    /// The header magic did not match [`HEADER_MAGIC`].
    #[error(
        "trace header magic mismatch: got 0x{got:08x}, expected 0x{:08x}",
        HEADER_MAGIC
    )]
    BadHeaderMagic {
        /// Magic value the trace carried.
        got: u32,
    },
    /// The header version differed from [`FORMAT_VERSION`].
    #[error(
        "trace version {got} unsupported (this build expects {})",
        FORMAT_VERSION
    )]
    BadVersion {
        /// Version the trace declared.
        got: u32,
    },
    /// A record name is longer than the reader's cap.
    #[error("record name length {len} exceeds 1 KiB sanity cap")]
    NameTooLong {
        /// Declared name length.
        len: u32,
    },
    /// A write payload is larger than the reader's cap.
    #[error("write payload size {size} exceeds 1 MiB sanity cap")]
    WriteTooLarge {
        /// Declared payload size.
        size: u32,
    },
    /// The trace ended inside a field.
    #[error("unexpected EOF while reading {in_field}")]
    UnexpectedEof {
        /// Field being read.
        in_field: &'static str,
    },
}

impl HleTraceError {
    /// Whether the reader can resume after this error. An I/O failure
    /// while searching for a record magic is fatal. A record-body
    /// failure, an I/O failure inside a body included, reads as
    /// [`HleTraceError::UnexpectedEof`]: the reader drops the record
    /// and resumes at the next record magic.
    pub fn is_fatal(&self) -> bool {
        match self {
            HleTraceError::Io(_) => true,
            HleTraceError::BadHeaderMagic { .. }
            | HleTraceError::BadVersion { .. }
            | HleTraceError::NameTooLong { .. }
            | HleTraceError::WriteTooLarge { .. }
            | HleTraceError::UnexpectedEof { .. } => false,
        }
    }
}

/// One step of an HLE trace read.
#[derive(Debug)]
pub enum HleTraceEvent {
    /// A record that decoded.
    Record(HleCallRecord),
    /// This many bytes were skipped while searching for the next record
    /// magic.
    SkippedBytes(usize),
    /// A record whose body did not decode; the reader resumes at the
    /// next record magic.
    DroppedRecord(HleTraceError),
}

/// Streaming reader over an HLE trace.
///
/// Yields one [`HleTraceEvent`] per record, per run of skipped bytes,
/// and per dropped record, then ends at end of input. A trailing
/// fragment shorter than a record magic ends the trace without an
/// event. An I/O failure while searching for a record magic yields
/// `Err`, after which the reader ends; bytes skipped since the last
/// event go unreported.
pub struct HleTraceReader<R: Read> {
    reader: R,
    window: [u8; 4],
    have_window: bool,
    skipped: usize,
    done: bool,
}

impl<R: Read> HleTraceReader<R> {
    /// Read and check the header.
    ///
    /// # Errors
    ///
    /// [`HleTraceError::BadHeaderMagic`], [`HleTraceError::BadVersion`],
    /// or [`HleTraceError::UnexpectedEof`] for a header cut short.
    pub fn new(mut reader: R) -> Result<Self, HleTraceError> {
        let header_magic = read_u32(&mut reader, "header magic")?;
        if header_magic != HEADER_MAGIC {
            return Err(HleTraceError::BadHeaderMagic { got: header_magic });
        }
        let version = read_u32(&mut reader, "trace version")?;
        if version != FORMAT_VERSION {
            return Err(HleTraceError::BadVersion { got: version });
        }
        Ok(Self {
            reader,
            window: [0; 4],
            have_window: false,
            skipped: 0,
            done: false,
        })
    }

    /// End the read, reporting any bytes skipped since the last event.
    fn finish(&mut self) -> Option<Result<HleTraceEvent, HleTraceError>> {
        self.done = true;
        let skipped = std::mem::take(&mut self.skipped);
        (skipped > 0).then_some(Ok(HleTraceEvent::SkippedBytes(skipped)))
    }
}

impl<R: Read> Iterator for HleTraceReader<R> {
    type Item = Result<HleTraceEvent, HleTraceError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        loop {
            if !self.have_window {
                // Read up to 4 bytes, looping on partial reads, so a
                // buffer boundary in the reader cannot pass for EOF; a
                // real EOF returns 0 from the first read.
                let mut filled = 0usize;
                while filled < 4 {
                    let n = match self.reader.read(&mut self.window[filled..]) {
                        Ok(n) => n,
                        Err(error) => {
                            self.done = true;
                            return Some(Err(HleTraceError::Io(error)));
                        }
                    };
                    if n == 0 {
                        break;
                    }
                    filled += n;
                }
                if filled < 4 {
                    // A clean end on a record boundary, or trailing
                    // bytes too short for a magic.
                    return self.finish();
                }
                self.have_window = true;
            }
            if u32::from_le_bytes(self.window) != RECORD_MAGIC {
                // Slide the window left by one byte and read one fresh.
                self.window.copy_within(1.., 0);
                let mut one = [0u8; 1];
                match self.reader.read(&mut one) {
                    Ok(0) => return self.finish(),
                    Ok(_) => {}
                    Err(error) => {
                        self.done = true;
                        return Some(Err(HleTraceError::Io(error)));
                    }
                }
                self.window[3] = one[0];
                self.skipped += 1;
                continue;
            }
            if self.skipped > 0 {
                // Report the skipped run first; the window still holds
                // the magic, so the next call decodes its record.
                let skipped = std::mem::take(&mut self.skipped);
                return Some(Ok(HleTraceEvent::SkippedBytes(skipped)));
            }
            self.have_window = false;
            return Some(match parse_one_record(&mut self.reader) {
                Ok(record) => Ok(HleTraceEvent::Record(record)),
                Err(error) if error.is_fatal() => {
                    self.done = true;
                    Err(error)
                }
                Err(error) => Ok(HleTraceEvent::DroppedRecord(error)),
            });
        }
    }
}

fn parse_one_record<R: Read>(reader: &mut R) -> Result<HleCallRecord, HleTraceError> {
    let step = read_u64(reader, "step")?;
    let lr = read_u64(reader, "lr")?;
    let thread_id = read_u32(reader, "thread_id")?;
    let depth = read_u32(reader, "depth")?;
    let name_len = read_u32(reader, "name_len")?;
    if name_len > NAME_LEN_CAP {
        return Err(HleTraceError::NameTooLong { len: name_len });
    }
    let mut name_bytes = vec![0u8; name_len as usize];
    reader
        .read_exact(&mut name_bytes)
        .map_err(|_| HleTraceError::UnexpectedEof { in_field: "name" })?;
    let name = String::from_utf8_lossy(&name_bytes).into_owned();

    let mut args = [0u64; 8];
    for slot in &mut args {
        *slot = read_u64(reader, "args[i]")?;
    }
    let ret = read_u64(reader, "ret")?;

    let num_writes = read_u32(reader, "num_writes")?;
    let mut writes = Vec::new();
    for _ in 0..num_writes {
        let addr = read_u64(reader, "write.addr")?;
        let size = read_u32(reader, "write.size")?;
        if size > WRITE_SIZE_CAP {
            return Err(HleTraceError::WriteTooLarge { size });
        }
        let mut bytes = vec![0u8; size as usize];
        reader
            .read_exact(&mut bytes)
            .map_err(|_| HleTraceError::UnexpectedEof {
                in_field: "write.bytes",
            })?;
        writes.push(HleWrite { addr, bytes });
    }

    Ok(HleCallRecord {
        step,
        lr,
        thread_id,
        depth,
        name,
        args,
        ret,
        writes,
    })
}

fn read_u32<R: Read>(r: &mut R, field: &'static str) -> Result<u32, HleTraceError> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)
        .map_err(|_| HleTraceError::UnexpectedEof { in_field: field })?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64<R: Read>(r: &mut R, field: &'static str) -> Result<u64, HleTraceError> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)
        .map_err(|_| HleTraceError::UnexpectedEof { in_field: field })?;
    Ok(u64::from_le_bytes(buf))
}

/// Calls and writes per HLE function name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HleWriteTally {
    by_name: BTreeMap<String, (usize, usize)>,
}

/// One name's row in an [`HleWriteTally`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HleTallyRow<'a> {
    /// HLE function name.
    pub name: &'a str,
    /// Records naming it.
    pub calls: usize,
    /// Writes those records carry.
    pub writes: usize,
}

impl HleWriteTally {
    /// Count one record.
    pub fn add(&mut self, record: &HleCallRecord) {
        let entry = self.by_name.entry(record.name.clone()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += record.writes.len();
    }

    /// Every name, most writes first, then most calls, then by name.
    pub fn ranked(&self) -> Vec<HleTallyRow<'_>> {
        let mut rows: Vec<HleTallyRow<'_>> = self
            .by_name
            .iter()
            .map(|(name, &(calls, writes))| HleTallyRow {
                name,
                calls,
                writes,
            })
            .collect();
        rows.sort_by(|a, b| b.writes.cmp(&a.writes).then(b.calls.cmp(&a.calls)));
        rows
    }
}

#[cfg(test)]
#[path = "tests/rpcs3_hle_trace_tests.rs"]
mod tests;
