//! Reader for the binary trace file the patched RPCS3 emits via
//! `bridges/rpcs3-patch/files/Emu/Cell/cellgov_ppu_trace.{h,cpp}`.
//! See that header for the on-disk format spec.
//!
//! All multi-byte integers are little-endian. Vector lane bytes are
//! stored in the spec's byte-0-MSB order; the reader copies them
//! verbatim into the `u128` representation that
//! [`crate::state::PpuState::vr`] uses (`u128::from_be_bytes`).

use cellgov_ps3_abi::hardware::{FPR_COUNT, GPR_COUNT, VR_COUNT};
use thiserror::Error;

use super::{InstructionCase, MemorySnapshot, OracleSource, PpuStateSnapshot};

/// Header magic written first to a CellGov PPU trace file. Matches
/// the C++ constant in `cellgov_ppu_trace.h`.
pub const HEADER_MAGIC: u32 = 0xC0E6_0003;

/// Per-record magic. Matches the C++ constant in `cellgov_ppu_trace.h`.
pub const RECORD_MAGIC: u32 = 0xC0E6_0004;

/// Format version this reader understands.
pub const FORMAT_VERSION: u32 = 3;

/// Parse error class for [`read_trace`] and [`read_trace_bytes`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Rpcs3CaptureError {
    /// Input ran out before the requested field could be read.
    #[error("rpcs3_capture: unexpected EOF at offset {offset} reading {field}")]
    UnexpectedEof {
        /// Byte offset where the read was attempted.
        offset: usize,
        /// Field name that triggered the failure.
        field: &'static str,
    },
    /// Header magic byte sequence did not match [`HEADER_MAGIC`].
    #[error("rpcs3_capture: header magic mismatch (got 0x{got:08x}, expected 0x{expected:08x})")]
    HeaderMagic {
        /// Magic value the file carried.
        got: u32,
        /// Expected magic, copied here for the error display.
        expected: u32,
    },
    /// Header version differed from [`FORMAT_VERSION`].
    #[error("rpcs3_capture: format version mismatch (got {got}, expected {expected})")]
    Version {
        /// Version the file declared.
        got: u32,
        /// Version this reader supports.
        expected: u32,
    },
    /// A record's magic did not match [`RECORD_MAGIC`].
    #[error("rpcs3_capture: record magic mismatch at offset {offset} (got 0x{got:08x})")]
    RecordMagic {
        /// Byte offset of the misaligned record header.
        offset: usize,
        /// Magic value found at that offset.
        got: u32,
    },
    /// `mem_len` would push the record past the end of the buffer.
    #[error("rpcs3_capture: oversize memory window at offset {offset} (mem_len={mem_len})")]
    OversizeMem {
        /// Byte offset of the record.
        offset: usize,
        /// Length the record declared.
        mem_len: u32,
    },
}

/// Cursor-style byte reader; tracks offset for error messages.
struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.offset
    }

    fn read_u32(&mut self, field: &'static str) -> Result<u32, Rpcs3CaptureError> {
        if self.remaining() < 4 {
            return Err(Rpcs3CaptureError::UnexpectedEof {
                offset: self.offset,
                field,
            });
        }
        let v = u32::from_le_bytes([
            self.bytes[self.offset],
            self.bytes[self.offset + 1],
            self.bytes[self.offset + 2],
            self.bytes[self.offset + 3],
        ]);
        self.offset += 4;
        Ok(v)
    }

    fn read_u64(&mut self, field: &'static str) -> Result<u64, Rpcs3CaptureError> {
        if self.remaining() < 8 {
            return Err(Rpcs3CaptureError::UnexpectedEof {
                offset: self.offset,
                field,
            });
        }
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&self.bytes[self.offset..self.offset + 8]);
        self.offset += 8;
        Ok(u64::from_le_bytes(buf))
    }

    fn read_bytes(&mut self, n: usize, field: &'static str) -> Result<&'a [u8], Rpcs3CaptureError> {
        if self.remaining() < n {
            return Err(Rpcs3CaptureError::UnexpectedEof {
                offset: self.offset,
                field,
            });
        }
        let out = &self.bytes[self.offset..self.offset + n];
        self.offset += n;
        Ok(out)
    }
}

/// Read one state plus its trailing `rtime` (`ppu_thread::rtime`).
/// `rtime` lives on [`CapturedRecord`], not [`PpuStateSnapshot`].
fn read_state(cursor: &mut Cursor<'_>) -> Result<(PpuStateSnapshot, u64), Rpcs3CaptureError> {
    use cellgov_sync::ReservedLine;

    let mut state = PpuStateSnapshot::zero();
    for i in 0..GPR_COUNT {
        state.gpr[i] = cursor.read_u64("gpr")?;
    }
    for i in 0..FPR_COUNT {
        state.fpr[i] = cursor.read_u64("fpr")?;
    }
    for i in 0..VR_COUNT {
        let bytes = cursor.read_bytes(16, "vr")?;
        let mut arr = [0u8; 16];
        arr.copy_from_slice(bytes);
        state.vr[i] = u128::from_be_bytes(arr);
    }
    state.cr = cursor.read_u32("cr")?;
    state.lr = cursor.read_u64("lr")?;
    state.ctr = cursor.read_u64("ctr")?;
    state.xer = cursor.read_u64("xer")?;
    // `ppu_thread::raddr` is a u32; 0 means no reservation. The
    // reserved line is the 128-byte-aligned line containing it.
    let raddr = cursor.read_u32("raddr")?;
    state.reservation = if raddr == 0 {
        None
    } else {
        Some(ReservedLine::containing(raddr as u64))
    };
    let rtime = cursor.read_u64("rtime")?;
    Ok((state, rtime))
}

/// One decoded record from a CellGov PPU trace dump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedRecord {
    /// PC at instruction entry.
    pub pc: u64,
    /// 32-bit PPC instruction word as stored in guest memory (big-endian).
    pub raw_instruction: u32,
    /// RPCS3 PPU thread id that executed the instruction.
    pub thread_id: u32,
    /// State before the instruction.
    pub pre_state: PpuStateSnapshot,
    /// State after the instruction.
    pub post_state: PpuStateSnapshot,
    /// RPCS3's reservation acquire timestamp (`ppu_thread::rtime`)
    /// before the instruction. RPCS3-only diagnostic; CellGov does
    /// not model rtime.
    pub pre_reservation_rtime: u64,
    /// RPCS3's reservation acquire timestamp after the instruction.
    pub post_reservation_rtime: u64,
    /// Start address of the memory window the record covers (0 if
    /// the instruction touched no memory).
    pub mem_addr: u64,
    /// Memory bytes before the instruction.
    pub mem_pre: Vec<u8>,
    /// Memory bytes after the instruction.
    pub mem_post: Vec<u8>,
}

impl CapturedRecord {
    /// Convert this record into an [`InstructionCase`] tagged
    /// [`OracleSource::Rpcs3Capture`] with the given `capture_id`.
    pub fn to_instruction_case(
        &self,
        label: impl Into<String>,
        capture_id: &'static str,
    ) -> InstructionCase {
        InstructionCase {
            label: label.into(),
            initial_state: self.pre_state.clone(),
            initial_memory: MemorySnapshot {
                base: self.mem_addr,
                bytes: self.mem_pre.clone(),
            },
            raw_instruction: self.raw_instruction,
            expected_state: self.post_state.clone(),
            expected_memory: MemorySnapshot {
                base: self.mem_addr,
                bytes: self.mem_post.clone(),
            },
            source: OracleSource::Rpcs3Capture { capture_id },
        }
    }
}

/// Parse an in-memory byte slice as a CellGov PPU trace dump.
pub fn read_trace_bytes(bytes: &[u8]) -> Result<Vec<CapturedRecord>, Rpcs3CaptureError> {
    let mut cursor = Cursor::new(bytes);

    let magic = cursor.read_u32("header_magic")?;
    if magic != HEADER_MAGIC {
        return Err(Rpcs3CaptureError::HeaderMagic {
            got: magic,
            expected: HEADER_MAGIC,
        });
    }
    let version = cursor.read_u32("header_version")?;
    if version != FORMAT_VERSION {
        return Err(Rpcs3CaptureError::Version {
            got: version,
            expected: FORMAT_VERSION,
        });
    }

    let mut records = Vec::new();
    while cursor.remaining() > 0 {
        let record_offset = cursor.offset;
        let rmagic = cursor.read_u32("record_magic")?;
        if rmagic != RECORD_MAGIC {
            return Err(Rpcs3CaptureError::RecordMagic {
                offset: record_offset,
                got: rmagic,
            });
        }
        let pc = cursor.read_u64("pc")?;
        let raw = cursor.read_u32("raw_instruction")?;
        let thread_id = cursor.read_u32("thread_id")?;

        let (pre_state, pre_reservation_rtime) = read_state(&mut cursor)?;
        let (post_state, post_reservation_rtime) = read_state(&mut cursor)?;

        let mem_addr = cursor.read_u64("mem_addr")?;
        let mem_len = cursor.read_u32("mem_len")?;
        if mem_len as usize > cursor.remaining() / 2 {
            // mem_pre + mem_post must both fit.
            return Err(Rpcs3CaptureError::OversizeMem {
                offset: record_offset,
                mem_len,
            });
        }
        let mem_pre = cursor.read_bytes(mem_len as usize, "mem_pre")?.to_vec();
        let mem_post = cursor.read_bytes(mem_len as usize, "mem_post")?.to_vec();

        records.push(CapturedRecord {
            pc,
            raw_instruction: raw,
            thread_id,
            pre_state,
            post_state,
            pre_reservation_rtime,
            post_reservation_rtime,
            mem_addr,
            mem_pre,
            mem_post,
        });
    }

    Ok(records)
}

/// Read a CellGov PPU trace dump from a host path.
///
/// # Errors
///
/// Wraps [`std::io::Error`] as [`Rpcs3CaptureError::UnexpectedEof`]
/// for missing files; format errors surface as their specific
/// variants per [`read_trace_bytes`].
pub fn read_trace(path: &std::path::Path) -> Result<Vec<CapturedRecord>, Rpcs3CaptureError> {
    let bytes = std::fs::read(path).map_err(|_| Rpcs3CaptureError::UnexpectedEof {
        offset: 0,
        field: "file",
    })?;
    read_trace_bytes(&bytes)
}

#[cfg(test)]
#[path = "tests/rpcs3_capture_tests.rs"]
mod tests;
