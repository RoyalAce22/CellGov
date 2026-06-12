//! Instruction-stream emitter for the `disasm` subcommand.
//!
//! Owns vaddr-to-file-offset resolution (`select_segment`) over the
//! validated `PtLoad` list and the per-instruction print loop
//! (`disassemble`). Real instruction lines go to stdout; past-segment
//! markers, decode-error notes, and the data-not-code heuristic go to
//! stderr so a downstream tool can pipe stdout cleanly. Caller
//! supplies the segment list from [`super::elf::parse_pt_loads`];
//! this module trusts that producer-side validation and skips
//! per-byte bounds checks in the hot loop.

use std::io::{self, Write};

use cellgov_ppu::funcmap::FunctionMap;

use super::args::MAX_COUNT;
use super::elf::PtLoad;

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
    VaddrNotInPtLoad { vaddr: u64, segments: Vec<PtLoad> },
    #[error(
        "vaddr 0x{vaddr:016x} is in PT_LOAD vaddr=0x{:016x}+filesz=0x{:x} (memsz=0x{:x}) but past the file-backed range; nothing to disassemble (BSS / zero-fill)",
        seg.vaddr, seg.filesz, seg.memsz
    )]
    VaddrInBssOnly { vaddr: u64, seg: PtLoad },
}

impl DisasmError {
    pub(super) fn message(&self) -> String {
        self.to_string()
    }
}

fn render_vaddr_not_in_pt_load(vaddr: u64, segments: &[PtLoad]) -> String {
    use std::fmt::Write as _;
    let mut s = format!("vaddr 0x{vaddr:016x} not in any PT_LOAD; segments:");
    for seg in segments {
        let _ = write!(
            s,
            "\n  vaddr=0x{:016x}+filesz=0x{:x} memsz=0x{:x} file=0x{:x}",
            seg.vaddr, seg.filesz, seg.memsz, seg.offset
        );
    }
    s
}

/// Pick the PT_LOAD that file-backs `vaddr`. With overlapping
/// segments, pick the smallest containing segment; ties break on
/// lowest `p_offset`, then lowest `p_vaddr`. Emits a stderr note
/// when more than one segment matches.
fn select_segment(segments: &[PtLoad], vaddr: u64) -> Result<PtLoad, DisasmError> {
    // saturating_add guards against hand-rolled test PtLoads whose
    // vaddr range overflows u64; parse_pt_loads rejects those upstream.
    let mut candidates: Vec<PtLoad> = segments
        .iter()
        .copied()
        .filter(|s| vaddr >= s.vaddr && vaddr < s.vaddr.saturating_add(s.filesz))
        .collect();
    if candidates.is_empty() {
        let bss_match = segments
            .iter()
            .copied()
            .find(|s| vaddr >= s.vaddr && vaddr < s.vaddr.saturating_add(s.memsz));
        if let Some(seg) = bss_match {
            return Err(DisasmError::VaddrInBssOnly { vaddr, seg });
        }
        return Err(DisasmError::VaddrNotInPtLoad {
            vaddr,
            segments: segments.to_vec(),
        });
    }
    if candidates.len() > 1 {
        eprintln!(
            "note: vaddr 0x{vaddr:x} is in {} overlapping PT_LOADs; choosing the smallest containing segment",
            candidates.len()
        );
    }
    candidates.sort_by_key(|s| (s.filesz, s.offset, s.vaddr));
    Ok(candidates[0])
}

/// Read `count` aligned 32-bit words starting at `vaddr`, decoding
/// each and writing one line per word into `out`.
///
/// Cross-module contract: `parse_pt_loads` must have validated the
/// segments. `seg.offset + off_in_seg + 4 <= elf_bytes.len()` whenever
/// the per-iteration filesz check passes, so the hot loop indexes
/// `elf_bytes` without further bounds checks.
pub(super) fn disassemble<W: Write>(
    elf_bytes: &[u8],
    segments: &[PtLoad],
    vaddr: u64,
    count: usize,
    symbols: Option<&FunctionMap>,
    out: &mut W,
) -> Result<DisasmStats, StreamError> {
    debug_assert!(vaddr.is_multiple_of(4), "parse_args must enforce alignment");
    debug_assert!(count > 0, "parse_args must reject count == 0");
    debug_assert!(count <= MAX_COUNT, "parse_args must enforce the count cap");

    let seg = select_segment(segments, vaddr).map_err(StreamError::BadVaddr)?;

    let mut stats = DisasmStats::default();
    let mut consecutive = 0usize;

    for n in 0..count {
        let Some(addr) = (n as u64)
            .checked_mul(4)
            .and_then(|delta| vaddr.checked_add(delta))
        else {
            writeln!(out, "<address overflow: vaddr+4*{n} exceeds u64::MAX>")
                .map_err(StreamError::Io)?;
            stats.lines_written += 1;
            stats.markers_written += 1;
            break;
        };
        let off_in_seg = addr - seg.vaddr;
        let needed_end = off_in_seg.checked_add(4);
        match needed_end {
            Some(end) if end <= seg.filesz => {}
            Some(end) if end <= seg.memsz => {
                writeln!(
                    out,
                    "0x{addr:016x}  --------  <in PT_LOAD but past filesz (BSS / zero-fill)>"
                )
                .map_err(StreamError::Io)?;
                stats.lines_written += 1;
                stats.markers_written += 1;
                break;
            }
            _ => {
                writeln!(out, "0x{addr:016x}  --------  <past segment end>")
                    .map_err(StreamError::Io)?;
                stats.lines_written += 1;
                stats.markers_written += 1;
                break;
            }
        }
        let file_off = (seg.offset + off_in_seg) as usize;
        let raw = u32::from_be_bytes([
            elf_bytes[file_off],
            elf_bytes[file_off + 1],
            elf_bytes[file_off + 2],
            elf_bytes[file_off + 3],
        ]);
        // Function separator when the stream crosses a span start.
        if let Some(map) = symbols {
            if let Ok(addr32) = u32::try_from(addr) {
                if let Some(span) = map.span_at(addr32) {
                    if span.start == addr32 {
                        writeln!(
                            out,
                            "; -- function {} ({}) --",
                            span.display_name(),
                            span.origin.as_str()
                        )
                        .map_err(StreamError::Io)?;
                    }
                }
            }
        }
        match cellgov_ppu::decode::decode(raw) {
            Ok(insn) => {
                consecutive = 0;
                let text = cellgov_ppu::instruction::AsmText {
                    insn: &insn,
                    addr,
                    symbols,
                };
                writeln!(out, "0x{addr:016x}  {raw:08x}  {text}").map_err(StreamError::Io)?;
                stats.lines_written += 1;
            }
            Err(_) => {
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
        }
    }
    Ok(stats)
}

#[cfg(test)]
#[path = "tests/stream_tests.rs"]
mod tests;
