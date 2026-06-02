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
        match cellgov_ppu::decode::decode(raw) {
            Ok(insn) => {
                consecutive = 0;
                writeln!(out, "0x{addr:016x}  {raw:08x}  {insn:?}").map_err(StreamError::Io)?;
                stats.lines_written += 1;
            }
            Err(_) => {
                consecutive += 1;
                stats.decode_errors += 1;
                writeln!(out, "0x{addr:016x}  {raw:08x}  <unsupported encoding>")
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
mod tests {
    use super::*;
    use crate::disasm::elf::parse_pt_loads;
    use crate::disasm::test_support::*;

    fn nop_elf() -> (Vec<u8>, Vec<PtLoad>) {
        let mut bytes = Vec::new();
        for _ in 0..4 {
            bytes.extend_from_slice(&PPC_NOP_BYTES);
        }
        bytes.extend_from_slice(&PPC_BLR_BYTES);
        let data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, bytes)]);
        let segs = parse_pt_loads(&data).unwrap();
        (data, segs)
    }

    #[test]
    fn disassemble_decodes_aligned_words() {
        let (data, segs) = nop_elf();
        let mut out = Vec::new();
        let stats = disassemble(&data, &segs, 0x10000, 5, &mut out).unwrap();
        assert_eq!(stats.decode_errors, 0);
        assert_eq!(stats.lines_written, 5);
        assert_eq!(stats.markers_written, 0);
        assert!(
            stats.markers_written <= 1,
            "markers_written must never exceed 1"
        );
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("60000000"), "stream missing nop word: {text}");
        assert!(text.contains("4e800020"), "stream missing blr word: {text}");
    }

    #[test]
    fn disassemble_address_column_uses_16_hex_digits() {
        let (data, segs) = nop_elf();
        let mut out = Vec::new();
        disassemble(&data, &segs, 0x10000, 1, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.starts_with("0x0000000000010000"),
            "expected zero-padded 16-hex-digit address, got: {text}"
        );
    }

    #[test]
    fn disassemble_rejects_vaddr_outside_pt_load() {
        let (data, segs) = nop_elf();
        let mut out = Vec::new();
        let err = disassemble(&data, &segs, 0x90000, 1, &mut out).unwrap_err();
        match err {
            StreamError::BadVaddr(DisasmError::VaddrNotInPtLoad { vaddr: 0x90000, .. }) => {}
            other => panic!("expected BadVaddr/VaddrNotInPtLoad, got {other:?}"),
        }
    }

    #[test]
    fn disassemble_distinguishes_bss_from_outside_segment() {
        let mut spec = SegSpec::pt_load(0x200, 0x10000, PPC_NOP_BYTES.to_vec());
        spec.p_memsz = 12;
        let data = build_elf64_be(&[spec]);
        let segs = parse_pt_loads(&data).unwrap();
        let mut out = Vec::new();
        let err = disassemble(&data, &segs, 0x10004, 1, &mut out).unwrap_err();
        match err {
            StreamError::BadVaddr(DisasmError::VaddrInBssOnly { vaddr: 0x10004, .. }) => {}
            other => panic!("expected BadVaddr/VaddrInBssOnly, got {other:?}"),
        }
    }

    #[test]
    fn disassemble_marks_bss_when_iterating_into_zero_fill() {
        let mut spec = SegSpec::pt_load(0x200, 0x10000, PPC_NOP_BYTES.to_vec());
        spec.p_memsz = 12;
        let data = build_elf64_be(&[spec]);
        let segs = parse_pt_loads(&data).unwrap();
        let mut out = Vec::new();
        let stats = disassemble(&data, &segs, 0x10000, 4, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("BSS / zero-fill"), "stream:\n{text}");
        assert_eq!(stats.lines_written, 2);
        assert_eq!(stats.markers_written, 1);
        assert!(
            stats.markers_written <= 1,
            "markers_written must never exceed 1"
        );
        assert_eq!(stats.decode_errors, 0);
    }

    #[test]
    fn disassemble_marks_past_segment_end_when_outside_memsz_too() {
        let (data, segs) = nop_elf();
        let mut out = Vec::new();
        let stats = disassemble(&data, &segs, 0x10000, 8, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("past segment end"), "stream:\n{text}");
        assert_eq!(stats.lines_written, 6);
        assert_eq!(stats.markers_written, 1);
        assert!(
            stats.markers_written <= 1,
            "markers_written must never exceed 1"
        );
    }

    /// Primary opcode 1 has no top-level arm in `cellgov_ppu::decode`,
    /// so any word of the form `0x04xxxxxx` returns
    /// `PpuDecodeError::Unsupported`. 0xFFFFFFFF would NOT work --
    /// primary 63 routes to `Fp63`.
    const UNSUPPORTED_WORD: [u8; 4] = [0x04, 0x00, 0x00, 0x00];

    #[test]
    fn disassemble_consecutive_decode_failures_emit_warning() {
        let mut bytes = Vec::new();
        for _ in 0..16 {
            bytes.extend_from_slice(&UNSUPPORTED_WORD);
        }
        let data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, bytes)]);
        let segs = parse_pt_loads(&data).unwrap();
        let mut out = Vec::new();
        let stats = disassemble(&data, &segs, 0x10000, 16, &mut out).unwrap();
        assert_eq!(stats.decode_errors, 16);
        assert!(stats.data_warning_emitted);
    }

    #[test]
    fn disassemble_resets_consecutive_counter_on_success() {
        let mut bytes = Vec::new();
        for _ in 0..4 {
            bytes.extend_from_slice(&UNSUPPORTED_WORD);
        }
        for _ in 0..4 {
            bytes.extend_from_slice(&PPC_NOP_BYTES);
        }
        for _ in 0..4 {
            bytes.extend_from_slice(&UNSUPPORTED_WORD);
        }
        let data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, bytes)]);
        let segs = parse_pt_loads(&data).unwrap();
        let mut out = Vec::new();
        let stats = disassemble(&data, &segs, 0x10000, 12, &mut out).unwrap();
        assert_eq!(stats.decode_errors, 8);
        assert!(!stats.data_warning_emitted);
    }

    #[test]
    fn select_segment_picks_smallest_containing_when_overlapping() {
        let big = PtLoad {
            vaddr: 0x10000,
            offset: 0x200,
            filesz: 0x1000,
            memsz: 0x1000,
        };
        let small = PtLoad {
            vaddr: 0x10000,
            offset: 0x4000,
            filesz: 0x100,
            memsz: 0x100,
        };
        let chosen = select_segment(&[big, small], 0x10000).unwrap();
        assert_eq!(chosen, small);
    }

    #[test]
    fn select_segment_breaks_filesz_ties_by_offset_then_vaddr() {
        let a = PtLoad {
            vaddr: 0x10000,
            offset: 0x4000,
            filesz: 0x100,
            memsz: 0x100,
        };
        let b = PtLoad {
            vaddr: 0x10000,
            offset: 0x2000,
            filesz: 0x100,
            memsz: 0x100,
        };
        let c = PtLoad {
            vaddr: 0x10000,
            offset: 0x6000,
            filesz: 0x100,
            memsz: 0x100,
        };
        let chosen = select_segment(&[a, b, c], 0x10000).unwrap();
        assert_eq!(chosen, b);
    }

    #[test]
    fn unsupported_word_constant_is_actually_unsupported() {
        let raw = u32::from_be_bytes(UNSUPPORTED_WORD);
        assert!(
            cellgov_ppu::decode::decode(raw).is_err(),
            "UNSUPPORTED_WORD ({raw:#010x}) decoded successfully; pick a different sentinel"
        );
    }

    #[test]
    fn disassemble_decodes_last_word_in_segment() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&PPC_NOP_BYTES);
        bytes.extend_from_slice(&PPC_BLR_BYTES);
        let data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, bytes)]);
        let segs = parse_pt_loads(&data).unwrap();
        let mut out = Vec::new();
        let stats = disassemble(&data, &segs, 0x10004, 1, &mut out).unwrap();
        assert_eq!(stats.lines_written, 1);
        assert_eq!(stats.markers_written, 0);
        assert!(
            stats.markers_written <= 1,
            "markers_written must never exceed 1"
        );
        assert_eq!(stats.decode_errors, 0);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("4e800020"), "stream:\n{text}");
    }

    #[test]
    fn select_segment_rejects_vaddr_at_exclusive_upper_bound() {
        let mut bytes = Vec::new();
        for _ in 0..4 {
            bytes.extend_from_slice(&PPC_NOP_BYTES);
        }
        let data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, bytes)]);
        let segs = parse_pt_loads(&data).unwrap();
        let err = select_segment(&segs, 0x10010).unwrap_err();
        match err {
            DisasmError::VaddrNotInPtLoad { vaddr: 0x10010, .. } => {}
            other => panic!("expected VaddrNotInPtLoad, got {other:?}"),
        }
    }

    /// `Write` that returns `BrokenPipe` after `fail_after` successful
    /// writes. Tracks how many writes the loop attempted so a test can
    /// assert it stopped at the first failure.
    struct BreakingWriter {
        successes_before_break: usize,
        writes_attempted: usize,
    }

    impl Write for BreakingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.writes_attempted += 1;
            if self.writes_attempted > self.successes_before_break {
                return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn disassemble_propagates_broken_pipe() {
        let mut bytes = Vec::new();
        for _ in 0..16 {
            bytes.extend_from_slice(&PPC_NOP_BYTES);
        }
        let data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, bytes)]);
        let segs = parse_pt_loads(&data).unwrap();
        let mut out = BreakingWriter {
            successes_before_break: 2,
            writes_attempted: 0,
        };
        let err = disassemble(&data, &segs, 0x10000, 16, &mut out).unwrap_err();
        assert!(
            matches!(&err, StreamError::Io(e) if e.kind() == std::io::ErrorKind::BrokenPipe),
            "expected Io(BrokenPipe), got {err:?}"
        );
        assert_eq!(
            out.writes_attempted, 3,
            "loop kept writing after BrokenPipe"
        );
    }

    #[test]
    fn disassemble_address_overflow_marker_is_consistent() {
        // Non-obvious invariant: parse_pt_loads rejects this geometry
        // (vaddr+filesz overflows); hand-rolled here to reach the
        // defensive marker path.
        let bytes = PPC_NOP_BYTES.to_vec();
        let seg = PtLoad {
            vaddr: u64::MAX - 3,
            offset: 0,
            filesz: 4,
            memsz: 4,
        };
        let mut out = Vec::new();
        let stats = disassemble(&bytes, &[seg], u64::MAX - 3, 2, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("<address overflow"),
            "stream missing address-overflow marker:\n{text}"
        );
        assert_eq!(stats.lines_written, 2);
        assert_eq!(stats.markers_written, 1);
        assert!(
            stats.markers_written <= 1,
            "markers_written must never exceed 1"
        );
        assert_eq!(stats.decode_errors, 0);
    }

    #[test]
    fn select_segment_at_exclusive_filesz_upper_bound_with_bigger_memsz_is_bss() {
        let mut spec = SegSpec::pt_load(0x200, 0x10000, PPC_NOP_BYTES.to_vec());
        spec.p_memsz = 16;
        let data = build_elf64_be(&[spec]);
        let segs = parse_pt_loads(&data).unwrap();
        let err = select_segment(&segs, 0x10004).unwrap_err();
        match err {
            DisasmError::VaddrInBssOnly { vaddr: 0x10004, .. } => {}
            other => panic!("expected VaddrInBssOnly, got {other:?}"),
        }
    }
}
