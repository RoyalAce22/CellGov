use std::io::Write;

use super::args::MAX_COUNT;
use super::elf::PtLoad;

/// Number of consecutive `decode` failures after which the user almost
/// certainly pointed the disassembler at data, not code. One stderr
/// note per run.
const CONSECUTIVE_DECODE_NOTE_THRESHOLD: usize = 8;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct DisasmStats {
    /// Total lines written to the instruction stream, including the
    /// terminal `<past segment end>` / `<BSS / zero-fill>` markers.
    /// Real instruction lines are `lines_written - markers_written`.
    pub(super) lines_written: usize,
    /// Boundary marker lines (BSS or past-segment-end) written. At
    /// most one per run.
    pub(super) markers_written: usize,
    /// Number of `decode` failures encountered.
    pub(super) decode_errors: usize,
    /// True if the consecutive-failure heuristic note fired.
    pub(super) data_warning_emitted: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum DisasmError {
    VaddrNotInPtLoad { vaddr: u64, segments: Vec<PtLoad> },
    VaddrInBssOnly { vaddr: u64, seg: PtLoad },
}

impl DisasmError {
    pub(super) fn message(&self) -> String {
        match self {
            Self::VaddrNotInPtLoad { vaddr, segments } => {
                let segs = segments
                    .iter()
                    .map(|s| {
                        format!(
                            "\n  vaddr=0x{:x}+filesz=0x{:x} memsz=0x{:x} file=0x{:x}",
                            s.vaddr, s.filesz, s.memsz, s.offset
                        )
                    })
                    .collect::<String>();
                format!("vaddr 0x{vaddr:x} not in any PT_LOAD; segments:{segs}")
            }
            Self::VaddrInBssOnly { vaddr, seg } => format!(
                "vaddr 0x{vaddr:x} is in PT_LOAD vaddr=0x{:x}+filesz=0x{:x} (memsz=0x{:x}) but past the file-backed range; nothing to disassemble (BSS / zero-fill)",
                seg.vaddr, seg.filesz, seg.memsz
            ),
        }
    }
}

/// Pick the PT_LOAD that file-backs `vaddr`. With overlapping
/// segments (legal but unusual; firmware modules occasionally emit
/// them), pick the smallest containing segment; ties break on lowest
/// `p_offset`, then lowest `p_vaddr`. Two PT_LOADs with identical
/// `(filesz, offset, vaddr)` are indistinguishable by file content,
/// so picking either is correct -- the triple-key sort makes the
/// choice fully a function of the segment data, not phdr order.
/// Emits a stderr note when more than one segment matches.
fn select_segment(segments: &[PtLoad], vaddr: u64) -> Result<PtLoad, DisasmError> {
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
/// Caller is responsible for `vaddr % 4 == 0` and `count > 0`
/// (`parse_args` enforces both). `parse_pt_loads` guarantees that any
/// `seg.offset + off_in_seg + 4 <= elf_bytes.len()` whenever the
/// per-iteration filesz check passes, so the hot loop indexes
/// `elf_bytes` without further bounds checks.
pub(super) fn disassemble<W: Write>(
    elf_bytes: &[u8],
    segments: &[PtLoad],
    vaddr: u64,
    count: usize,
    out: &mut W,
) -> Result<DisasmStats, DisasmError> {
    debug_assert!(vaddr.is_multiple_of(4), "parse_args must enforce alignment");
    debug_assert!(count > 0, "parse_args must reject count == 0");
    debug_assert!(count <= MAX_COUNT, "parse_args must enforce the count cap");

    let seg = select_segment(segments, vaddr)?;

    let mut stats = DisasmStats::default();
    let mut consecutive = 0usize;

    for n in 0..count {
        let Some(addr) = (n as u64)
            .checked_mul(4)
            .and_then(|delta| vaddr.checked_add(delta))
        else {
            let _ = writeln!(out, "<address overflow at iteration {n}>");
            break;
        };
        let off_in_seg = addr - seg.vaddr;
        let needed_end = off_in_seg.checked_add(4);
        match needed_end {
            Some(end) if end <= seg.filesz => {}
            Some(end) if end <= seg.memsz => {
                let _ = writeln!(
                    out,
                    "0x{addr:016x}  --------  <in PT_LOAD but past filesz (BSS / zero-fill)>"
                );
                stats.lines_written += 1;
                stats.markers_written += 1;
                break;
            }
            _ => {
                let _ = writeln!(out, "0x{addr:016x}  --------  <past segment end>");
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
                let _ = writeln!(out, "0x{addr:016x}  {raw:08x}  {insn:?}");
                stats.lines_written += 1;
            }
            Err(_) => {
                consecutive += 1;
                stats.decode_errors += 1;
                let _ = writeln!(out, "0x{addr:016x}  {raw:08x}  <unsupported encoding>");
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
    use super::super::elf::parse_pt_loads;
    use super::super::test_support::*;
    use super::*;

    fn nop_elf() -> (Vec<u8>, Vec<PtLoad>) {
        let mut bytes = Vec::new();
        for _ in 0..4 {
            bytes.extend_from_slice(&NOP);
        }
        bytes.extend_from_slice(&BLR);
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
            DisasmError::VaddrNotInPtLoad { vaddr: 0x90000, .. } => {}
            other => panic!("expected VaddrNotInPtLoad, got {other:?}"),
        }
    }

    #[test]
    fn disassemble_distinguishes_bss_from_outside_segment() {
        let mut spec = SegSpec::pt_load(0x200, 0x10000, NOP.to_vec());
        spec.p_memsz = 12;
        let data = build_elf64_be(&[spec]);
        let segs = parse_pt_loads(&data).unwrap();
        let mut out = Vec::new();
        let err = disassemble(&data, &segs, 0x10004, 1, &mut out).unwrap_err();
        match err {
            DisasmError::VaddrInBssOnly { vaddr: 0x10004, .. } => {}
            other => panic!("expected VaddrInBssOnly, got {other:?}"),
        }
    }

    #[test]
    fn disassemble_marks_bss_when_iterating_into_zero_fill() {
        let mut spec = SegSpec::pt_load(0x200, 0x10000, NOP.to_vec());
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
            bytes.extend_from_slice(&NOP);
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
        // If `cellgov_ppu::decode` ever adds an arm covering primary
        // opcode 1, the heuristic tests below silently flip from
        // "exercises the failure path" to "exercises the success
        // path." Fail loudly here so the maintainer picks a new
        // sentinel rather than letting the heuristic tests go vacuous.
        let raw = u32::from_be_bytes(UNSUPPORTED_WORD);
        assert!(
            cellgov_ppu::decode::decode(raw).is_err(),
            "UNSUPPORTED_WORD ({raw:#010x}) decoded successfully; pick a different sentinel"
        );
    }

    #[test]
    fn disassemble_decodes_last_word_in_segment() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&NOP);
        bytes.extend_from_slice(&BLR);
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
            bytes.extend_from_slice(&NOP);
        }
        let data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, bytes)]);
        let segs = parse_pt_loads(&data).unwrap();
        let err = select_segment(&segs, 0x10010).unwrap_err();
        match err {
            DisasmError::VaddrNotInPtLoad { vaddr: 0x10010, .. } => {}
            other => panic!("expected VaddrNotInPtLoad, got {other:?}"),
        }
    }

    #[test]
    fn select_segment_at_exclusive_filesz_upper_bound_with_bigger_memsz_is_bss() {
        let mut spec = SegSpec::pt_load(0x200, 0x10000, NOP.to_vec());
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
