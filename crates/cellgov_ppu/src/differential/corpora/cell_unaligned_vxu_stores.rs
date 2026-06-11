//! Spec-derived corpus for the Cell-unaligned VXU store family
//! (`stvlx`, `stvrx`, `stvlxl`, `stvrxl`).
//!
//! `stvlx` writes the high `16 - (EA & 0xF)` bytes of VS starting at
//! EA; `stvrx` writes the low `EA & 0xF` bytes of VS at the aligned
//! line below EA. Used together at EA and EA+16, they store a full
//! unaligned 16-byte vector without crossing the 16-byte aligned
//! line boundary mid-instruction (the architectural property that
//! gives the Cell PPE unaligned vector access without the AltiVec
//! `lvsl`/`vperm` shuffle).
//!
//! The "Last" suffixed twins (`stvlxl` / `stvrxl`) carry an LRU
//! cache hint that CellGov's no-cache model ignores; the corpus
//! treats them as direct aliases of `stvlx` / `stvrx`.

use super::super::{InstructionCase, MemorySnapshot};
use super::case_keep_memory;
use crate::differential::PpuStateSnapshot;

/// Encode an X-form primary-31 instruction `(vs, ra, rb, xo)`.
fn xform(vs: u8, ra: u8, rb: u8, xo: u32) -> u32 {
    (31u32 << 26)
        | ((vs as u32 & 0x1F) << 21)
        | ((ra as u32 & 0x1F) << 16)
        | ((rb as u32 & 0x1F) << 11)
        | (xo << 1)
}

const BASE_ADDR: u64 = 0x4000;

fn memory_with(bytes: Vec<u8>) -> MemorySnapshot {
    MemorySnapshot {
        base: BASE_ADDR,
        bytes,
    }
}

fn zero_memory(len: usize) -> MemorySnapshot {
    memory_with(vec![0u8; len])
}

/// State preloaded with RA, RB, and VS.
fn state_with_ra_rb_vs(
    ra_index: usize,
    ra_value: u64,
    rb_index: usize,
    rb_value: u64,
    vs_index: usize,
    vs_value: u128,
) -> PpuStateSnapshot {
    let mut s = PpuStateSnapshot::zero();
    s.gpr[ra_index] = ra_value;
    s.gpr[rb_index] = rb_value;
    s.vr[vs_index] = vs_value;
    s
}

/// Spec-derived cases for the entire Cell-unaligned VXU store family.
pub fn cases() -> Vec<InstructionCase> {
    let mut v = Vec::new();
    v.extend(stvlx_cases());
    v.extend(stvrx_cases());
    v.extend(stvlxl_cases());
    v.extend(stvrxl_cases());
    v
}

const VS_PATTERN: u128 = 0x0011_2233_4455_6677_8899_AABB_CCDD_EEFF;

/// `stvlx`: write the high `16 - m` bytes of VS starting at EA.
// [CBE-Handbook p:744 s:A.3.3] stvlx VS, RA, RB.
fn stvlx_cases() -> Vec<InstructionCase> {
    let raw = xform(/*vs*/ 11, /*ra*/ 4, /*rb*/ 5, 775);
    stvl_cases_for_xo(raw, "stvlx", VS_PATTERN, 11)
}

/// `stvrx`: write the low `m` bytes of VS at the aligned line below EA.
// [CBE-Handbook p:744 s:A.3.3] stvrx VS, RA, RB.
fn stvrx_cases() -> Vec<InstructionCase> {
    let raw = xform(12, 4, 5, 839);
    stvr_cases_for_xo(raw, "stvrx", VS_PATTERN, 12)
}

/// `stvlxl`: identical to stvlx + LRU hint.
fn stvlxl_cases() -> Vec<InstructionCase> {
    let raw = xform(13, 4, 5, 903);
    stvl_cases_for_xo(raw, "stvlxl", VS_PATTERN, 13)
}

/// `stvrxl`: identical to stvrx + LRU hint.
fn stvrxl_cases() -> Vec<InstructionCase> {
    let raw = xform(14, 4, 5, 967);
    stvr_cases_for_xo(raw, "stvrxl", VS_PATTERN, 14)
}

/// Shared case generator for stvlx / stvlxl. Covers EA offsets 0,
/// 4, 8, and 15 within the page -- the first three exercise common
/// alignments and the last tests the smallest possible count (1
/// byte).
fn stvl_cases_for_xo(
    raw: u32,
    prefix: &'static str,
    vs_pattern: u128,
    vs_index: usize,
) -> Vec<InstructionCase> {
    let vs_bytes = vs_pattern.to_be_bytes();
    let mut cases = Vec::new();
    for ea_offset in [0u64, 4, 8, 15] {
        let initial = state_with_ra_rb_vs(4, BASE_ADDR, 5, ea_offset, vs_index, vs_pattern);
        let expected_state = initial.clone();
        let m = (ea_offset & 0xF) as usize;
        let count = 16 - m;
        let mut expected_bytes = vec![0u8; 32];
        expected_bytes[ea_offset as usize..ea_offset as usize + count]
            .copy_from_slice(&vs_bytes[..count]);
        let label: &'static str = match (prefix, ea_offset) {
            ("stvlx", 0) => "stvlx_aligned_offset_0",
            ("stvlx", 4) => "stvlx_offset_4",
            ("stvlx", 8) => "stvlx_offset_8",
            ("stvlx", 15) => "stvlx_offset_15_one_byte",
            ("stvlxl", 0) => "stvlxl_aligned_offset_0",
            ("stvlxl", 4) => "stvlxl_offset_4",
            ("stvlxl", 8) => "stvlxl_offset_8",
            ("stvlxl", 15) => "stvlxl_offset_15_one_byte",
            _ => unreachable!(),
        };
        cases.push(case_keep_memory(
            label,
            raw,
            initial,
            zero_memory(32),
            expected_state,
            memory_with(expected_bytes),
            "[CBE-Handbook p:744 s:A.3.3] stvlx: VS[0..16-m] -> MEM(EA, 16-m); m=EA&0xF",
        ));
    }
    cases
}

/// Shared case generator for stvrx / stvrxl. EA offsets 0, 1, 8,
/// and 15 cover: zero bytes (aligned EA), single byte, half line,
/// and 15-byte tail.
fn stvr_cases_for_xo(
    raw: u32,
    prefix: &'static str,
    vs_pattern: u128,
    vs_index: usize,
) -> Vec<InstructionCase> {
    let vs_bytes = vs_pattern.to_be_bytes();
    let mut cases = Vec::new();
    for ea_offset in [0u64, 1, 8, 15] {
        let initial = state_with_ra_rb_vs(4, BASE_ADDR, 5, ea_offset, vs_index, vs_pattern);
        let expected_state = initial.clone();
        let m = (ea_offset & 0xF) as usize;
        let aligned_offset = (ea_offset as usize) - m;
        let mut expected_bytes = vec![0u8; 32];
        for i in 0..m {
            expected_bytes[aligned_offset + i] = vs_bytes[16 - m + i];
        }
        let label: &'static str = match (prefix, ea_offset) {
            ("stvrx", 0) => "stvrx_aligned_offset_0_zero_bytes",
            ("stvrx", 1) => "stvrx_offset_1_one_byte",
            ("stvrx", 8) => "stvrx_offset_8",
            ("stvrx", 15) => "stvrx_offset_15",
            ("stvrxl", 0) => "stvrxl_aligned_offset_0_zero_bytes",
            ("stvrxl", 1) => "stvrxl_offset_1_one_byte",
            ("stvrxl", 8) => "stvrxl_offset_8",
            ("stvrxl", 15) => "stvrxl_offset_15",
            _ => unreachable!(),
        };
        cases.push(case_keep_memory(
            label,
            raw,
            initial,
            zero_memory(32),
            expected_state,
            memory_with(expected_bytes),
            "[CBE-Handbook p:744 s:A.3.3] stvrx: VS[16-m..16] -> MEM(EA&~0xF, m); m=EA&0xF",
        ));
    }
    cases
}

#[cfg(test)]
mod tests {
    use super::super::super::{assert_case, run_corpus};
    use super::*;

    #[test]
    fn cell_unaligned_vxu_store_corpus_passes_against_executor() {
        let cases = cases();
        assert!(
            !cases.is_empty(),
            "Cell-unaligned VXU store corpus must produce at least one case"
        );
        let report = run_corpus(&cases);
        if !report.is_clean() {
            let detail = report
                .failed
                .iter()
                .map(|(label, outcome)| format!("  '{label}': {outcome:?}"))
                .collect::<Vec<_>>()
                .join("\n");
            panic!(
                "Cell-unaligned VXU store corpus: {} failure(s) of {}:\n{detail}",
                report.failed.len(),
                report.total()
            );
        }
    }

    #[test]
    fn each_case_passes_through_assert_case() {
        for case in cases() {
            assert_case(&case);
        }
    }

    #[test]
    fn corpus_covers_all_four_ops() {
        let cases = cases();
        let labels: Vec<&str> = cases.iter().map(|c| c.label).collect();
        for prefix in ["stvlx_", "stvrx_", "stvlxl_", "stvrxl_"] {
            assert!(
                labels.iter().any(|l| l.starts_with(prefix)),
                "corpus missing any '{prefix}' case"
            );
        }
    }

    #[test]
    fn stvlx_plus_stvrx_at_next_line_writes_full_vector() {
        use crate::differential::run_case;
        use crate::differential::CaseOutcome;

        let ea_offset: u64 = 5;
        let vs = VS_PATTERN;
        let vs_bytes = vs.to_be_bytes();

        // Stvlx at offset 5 writes bytes 0..11 of VS at [5..16].
        let raw_stvlx = xform(11, 4, 5, 775);
        let initial = state_with_ra_rb_vs(4, BASE_ADDR, 5, ea_offset, 11, vs);
        let mut expected_bytes = vec![0u8; 32];
        expected_bytes[5..16].copy_from_slice(&vs_bytes[..11]);
        let case_l = case_keep_memory(
            "stvlx_then_stvrx_part1",
            raw_stvlx,
            initial,
            zero_memory(32),
            state_with_ra_rb_vs(4, BASE_ADDR, 5, ea_offset, 11, vs),
            memory_with(expected_bytes),
            "[CBE-Handbook p:744 s:A.3.3] stvlx half of unaligned vector store pair",
        );
        assert_eq!(run_case(&case_l), CaseOutcome::Pass);

        // Stvrx at offset 5+16 = 21 writes bytes 11..16 of VS at
        // [16..21] (the next aligned line below EA = 21).
        let ea_offset_2: u64 = 5 + 16;
        let initial_2 = state_with_ra_rb_vs(4, BASE_ADDR, 5, ea_offset_2, 11, vs);
        let mut expected_bytes_2 = vec![0u8; 32];
        expected_bytes_2[16..21].copy_from_slice(&vs_bytes[11..]);
        let raw_stvrx = xform(11, 4, 5, 839);
        let case_r = case_keep_memory(
            "stvlx_then_stvrx_part2",
            raw_stvrx,
            initial_2,
            zero_memory(32),
            state_with_ra_rb_vs(4, BASE_ADDR, 5, ea_offset_2, 11, vs),
            memory_with(expected_bytes_2),
            "[CBE-Handbook p:744 s:A.3.3] stvrx half of unaligned vector store pair",
        );
        assert_eq!(run_case(&case_r), CaseOutcome::Pass);
    }
}
