//! Instruction-stream printer for `dev disasm`.
//!
//! `cellgov_ppu::loader::address_source` picks the segment a start
//! address sits in, and `cellgov_ppu::disasm::Disassembly` walks its
//! words; this module renders them. Instruction lines and the
//! end-of-stream marker go to stdout; the overlap note and the
//! data-not-code heuristic go to stderr so a downstream tool can pipe
//! stdout cleanly.

use std::io::{self, Write};

use cellgov_ppu::disasm::{DisasmEnd, DisasmItem, Disassembly};
use cellgov_ppu::funcmap::FunctionMap;
use cellgov_ppu::loader::{address_source, AddressSource, LoadSegment};

use crate::cli::parse::MAX_DISASM_COUNT as MAX_COUNT;

/// Number of consecutive `decode` failures after which the user almost
/// certainly pointed the disassembler at data, not code. One stderr
/// note per run.
const CONSECUTIVE_DECODE_NOTE_THRESHOLD: usize = 8;

#[derive(Debug, thiserror::Error)]
pub(super) enum StreamError {
    #[error("disasm: {0}")]
    BadVaddr(#[source] DisasmError),
    #[error("disasm I/O: {0}")]
    Io(#[source] io::Error),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct DisasmStats {
    /// Real instruction lines are `lines_written - markers_written`.
    pub(super) lines_written: usize,
    /// At most one per run.
    pub(super) markers_written: usize,
    pub(super) decode_errors: usize,
    pub(super) data_warning_emitted: bool,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(super) enum DisasmError {
    #[error("{}", render_vaddr_not_in_pt_load(*vaddr, segments))]
    VaddrNotInPtLoad {
        vaddr: u64,
        segments: Vec<LoadSegment>,
    },
    #[error(
        "vaddr 0x{vaddr:016x} is in PT_LOAD vaddr=0x{:016x}+filesz=0x{:x} (memsz=0x{:x}) but past the file-backed range; nothing to disassemble (BSS / zero-fill)",
        seg.vaddr, seg.filesz, seg.memsz
    )]
    VaddrInBssOnly { vaddr: u64, seg: LoadSegment },
}

impl DisasmError {
    pub(super) fn message(&self) -> String {
        self.to_string()
    }
}

fn render_vaddr_not_in_pt_load(vaddr: u64, segments: &[LoadSegment]) -> String {
    use std::fmt::Write as _;
    let mut s = format!("vaddr 0x{vaddr:016x} not in any PT_LOAD; segments:");
    for seg in segments {
        let _ = write!(
            s,
            "\n  vaddr=0x{:016x}+filesz=0x{:x} memsz=0x{:x} file=0x{:x}",
            seg.vaddr, seg.filesz, seg.memsz, seg.file_offset
        );
    }
    s
}

/// Pick the PT_LOAD that file-backs `vaddr`: the smallest containing
/// segment when several overlap (see
/// `cellgov_ppu::loader::address_source`). Emits a stderr note when
/// more than one segment matches.
fn select_segment(segments: &[LoadSegment], vaddr: u64) -> Result<LoadSegment, DisasmError> {
    match address_source(segments, vaddr, 1) {
        AddressSource::FileBacked {
            segment,
            overlapping,
            ..
        } => {
            if overlapping > 1 {
                eprintln!(
                    "note: vaddr 0x{vaddr:x} is in {overlapping} overlapping PT_LOADs; choosing the smallest containing segment"
                );
            }
            Ok(segment)
        }
        AddressSource::ZeroFill { segment } => Err(DisasmError::VaddrInBssOnly {
            vaddr,
            seg: segment,
        }),
        AddressSource::Unmapped => Err(DisasmError::VaddrNotInPtLoad {
            vaddr,
            segments: segments.to_vec(),
        }),
    }
}

/// Write one line per word for `count` aligned 32-bit words starting at
/// `vaddr`, and a marker line where the segment's file bytes end first.
pub(super) fn disassemble<W: Write>(
    elf_bytes: &[u8],
    segments: &[LoadSegment],
    vaddr: u64,
    count: usize,
    symbols: Option<&FunctionMap>,
    out: &mut W,
) -> Result<DisasmStats, StreamError> {
    debug_assert!(
        vaddr.is_multiple_of(4),
        "args::check_alignment must enforce alignment"
    );
    debug_assert!(count > 0, "the --count value parser must reject 0");
    debug_assert!(
        count <= MAX_COUNT,
        "the --count value parser must enforce the cap"
    );

    let seg = select_segment(segments, vaddr).map_err(StreamError::BadVaddr)?;

    let mut stats = DisasmStats::default();
    let mut consecutive = 0usize;
    let mut words = 0usize;
    let mut stream = Disassembly::new(elf_bytes, seg, vaddr, symbols);

    while words < count {
        let Some(item) = stream.next() else {
            break;
        };
        match item {
            DisasmItem::FunctionStart(span) => {
                writeln!(
                    out,
                    "; -- function {} ({}) --",
                    span.display_name(),
                    span.origin.as_str()
                )
                .map_err(StreamError::Io)?;
            }
            DisasmItem::Instruction { addr, raw, insn } => {
                words += 1;
                consecutive = 0;
                let text = cellgov_ppu::instruction::AsmText {
                    insn: &insn,
                    addr,
                    symbols,
                };
                writeln!(out, "0x{addr:016x}  {raw:08x}  {text}").map_err(StreamError::Io)?;
                stats.lines_written += 1;
            }
            DisasmItem::Undecodable { addr, raw } => {
                words += 1;
                consecutive += 1;
                stats.decode_errors += 1;
                // `.word` keeps the line greppable and parseable by
                // downstream tools.
                writeln!(out, "0x{addr:016x}  {raw:08x}  .word 0x{raw:08x}")
                    .map_err(StreamError::Io)?;
                stats.lines_written += 1;
                if !stats.data_warning_emitted && consecutive >= CONSECUTIVE_DECODE_NOTE_THRESHOLD {
                    eprintln!(
                        "note: {CONSECUTIVE_DECODE_NOTE_THRESHOLD}+ consecutive decode failures; this address may be data, not code"
                    );
                    stats.data_warning_emitted = true;
                }
            }
            DisasmItem::End(end) => {
                match end {
                    DisasmEnd::AddressOverflow { words: n } => {
                        writeln!(out, "<address overflow: vaddr+4*{n} exceeds u64::MAX>")
                    }
                    DisasmEnd::ZeroFill { addr } => writeln!(
                        out,
                        "0x{addr:016x}  --------  <in PT_LOAD but past filesz (BSS / zero-fill)>"
                    ),
                    DisasmEnd::SegmentEnd { addr } => {
                        writeln!(out, "0x{addr:016x}  --------  <past segment end>")
                    }
                }
                .map_err(StreamError::Io)?;
                stats.lines_written += 1;
                stats.markers_written += 1;
                break;
            }
        }
    }
    Ok(stats)
}

#[cfg(test)]
#[path = "tests/stream_tests.rs"]
mod tests;
