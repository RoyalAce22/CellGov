//! Disassembly stream output -- decode stats, address formatting, and out-of-segment vaddr errors.

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

/// Primary opcode 1 has no top-level arm in `cellgov_ppu::decode`
/// and no `known_encodings` row, so any word of the form
/// `0x04xxxxxx` returns `PpuDecodeError::EncodingNotRecognized`.
/// 0xFFFFFFFF would NOT work -- primary 63 routes to `Fp63`.
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
