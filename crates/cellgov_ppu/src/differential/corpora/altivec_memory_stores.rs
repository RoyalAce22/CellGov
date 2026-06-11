//! Spec-derived corpus for the AltiVec-memory store family (`stvebx`,
//! `stvehx`, `stvewx`, `stvxl`).
//!
//! Symmetric to the load family at
//! [`super::altivec_memory_loads`]: element-indexed stores derive
//! the source byte / halfword / word from a fixed-position lane of
//! VS (selected by EA mod 16), and EA itself is aligned down to the
//! element size before the store. `stvxl` is identical to `stvx`;
//! the LRU "Last" hint is a cache directive CellGov ignores.

use super::super::{InstructionCase, MemorySnapshot};
use super::case_keep_memory;
use crate::differential::PpuStateSnapshot;

/// Encode an X-form primary-31 instruction `(vt/vs, ra, rb, xo)`.
fn xform(vt_or_vs: u8, ra: u8, rb: u8, xo: u32) -> u32 {
    (31u32 << 26)
        | ((vt_or_vs as u32 & 0x1F) << 21)
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

/// Spec-derived cases for the entire AltiVec-memory store family.
pub fn cases() -> Vec<InstructionCase> {
    let mut v = Vec::new();
    v.extend(stvebx_cases());
    v.extend(stvehx_cases());
    v.extend(stvewx_cases());
    v.extend(stvxl_cases());
    v
}

/// `stvebx`: byte at lane (EA & 0xF) of VS -> MEM(EA, 1).
// [AltiVec-PEM p:6-29 s:6.2] stvebx VS, RA, RB.
fn stvebx_cases() -> Vec<InstructionCase> {
    let raw = xform(/*vs*/ 7, /*ra*/ 4, /*rb*/ 5, 135);
    let mut cases = Vec::new();

    let vs_pattern: u128 = 0x0011_2233_4455_6677_8899_AABB_CCDD_EEFF;
    let vs_bytes = vs_pattern.to_be_bytes();

    for ea_offset in [0u64, 1, 7, 15] {
        let initial = state_with_ra_rb_vs(4, BASE_ADDR, 5, ea_offset, 7, vs_pattern);
        let expected_state = initial.clone();
        let m = (ea_offset & 0xF) as usize;
        let mut expected_bytes = vec![0u8; 16];
        expected_bytes[ea_offset as usize] = vs_bytes[m];
        let label: &'static str = match ea_offset {
            0 => "stvebx_offset_0",
            1 => "stvebx_offset_1",
            7 => "stvebx_offset_7",
            15 => "stvebx_offset_15",
            _ => unreachable!(),
        };
        cases.push(case_keep_memory(
            label,
            raw,
            initial,
            zero_memory(16),
            expected_state,
            memory_with(expected_bytes),
            "[AltiVec-PEM p:6-29 s:6.2] byte at VS lane (EA&0xF) -> MEM(EA, 1)",
        ));
    }

    cases
}

/// `stvehx`: halfword at lane (EA & 0xE) of VS -> MEM(EA & ~1, 2).
// [AltiVec-PEM p:6-30 s:6.2] stvehx VS, RA, RB.
fn stvehx_cases() -> Vec<InstructionCase> {
    let raw = xform(8, 4, 5, 167);
    let mut cases = Vec::new();

    let vs_pattern: u128 = 0x1122_3344_5566_7788_99AA_BBCC_DDEE_FF00;
    let vs_bytes = vs_pattern.to_be_bytes();

    for ea_offset in [0u64, 2, 8, 14] {
        let initial = state_with_ra_rb_vs(4, BASE_ADDR, 5, ea_offset, 8, vs_pattern);
        let expected_state = initial.clone();
        let m = (ea_offset & 0xF) as usize;
        let mut expected_bytes = vec![0u8; 16];
        expected_bytes[ea_offset as usize] = vs_bytes[m];
        expected_bytes[ea_offset as usize + 1] = vs_bytes[m + 1];
        let label: &'static str = match ea_offset {
            0 => "stvehx_offset_0",
            2 => "stvehx_offset_2",
            8 => "stvehx_offset_8",
            14 => "stvehx_offset_14",
            _ => unreachable!(),
        };
        cases.push(case_keep_memory(
            label,
            raw,
            initial,
            zero_memory(16),
            expected_state,
            memory_with(expected_bytes),
            "[AltiVec-PEM p:6-30 s:6.2] halfword at VS lane (EA&0xE) -> MEM(EA&~1, 2)",
        ));
    }

    cases
}

/// `stvewx`: word at lane (EA & 0xC) of VS -> MEM(EA & ~3, 4).
// [AltiVec-PEM p:6-31 s:6.2] stvewx VS, RA, RB.
fn stvewx_cases() -> Vec<InstructionCase> {
    let raw = xform(9, 4, 5, 199);
    let mut cases = Vec::new();

    let vs_pattern: u128 = 0xDEAD_BEEF_CAFE_BABE_1234_5678_9ABC_DEF0;
    let vs_bytes = vs_pattern.to_be_bytes();

    for ea_offset in [0u64, 4, 8, 12] {
        let initial = state_with_ra_rb_vs(4, BASE_ADDR, 5, ea_offset, 9, vs_pattern);
        let expected_state = initial.clone();
        let m = (ea_offset & 0xF) as usize;
        let mut expected_bytes = vec![0u8; 16];
        expected_bytes[ea_offset as usize..ea_offset as usize + 4]
            .copy_from_slice(&vs_bytes[m..m + 4]);
        let label: &'static str = match ea_offset {
            0 => "stvewx_offset_0",
            4 => "stvewx_offset_4",
            8 => "stvewx_offset_8",
            12 => "stvewx_offset_12",
            _ => unreachable!(),
        };
        cases.push(case_keep_memory(
            label,
            raw,
            initial,
            zero_memory(16),
            expected_state,
            memory_with(expected_bytes),
            "[AltiVec-PEM p:6-31 s:6.2] word at VS lane (EA&0xC) -> MEM(EA&~3, 4)",
        ));
    }

    cases
}

/// `stvxl`: identical to stvx.
// [AltiVec-PEM p:6-33 s:6.2] stvxl VS, RA, RB.
fn stvxl_cases() -> Vec<InstructionCase> {
    let raw = xform(10, 4, 5, 487);
    let vs_pattern: u128 = 0xFFEE_DDCC_BBAA_9988_7766_5544_3322_1100;
    let vs_bytes = vs_pattern.to_be_bytes();
    let initial = state_with_ra_rb_vs(4, BASE_ADDR, 5, 0, 10, vs_pattern);
    let expected_state = initial.clone();
    let mut expected_bytes = vec![0u8; 32];
    expected_bytes[..16].copy_from_slice(&vs_bytes);
    vec![case_keep_memory(
        "stvxl_aligned_at_offset_0",
        raw,
        initial,
        zero_memory(32),
        expected_state,
        memory_with(expected_bytes),
        "[AltiVec-PEM p:6-33 s:6.2] stvxl: 16-byte aligned store; LRU hint ignored",
    )]
}

#[cfg(test)]
mod tests {
    use super::super::super::{assert_case, run_corpus};
    use super::*;

    #[test]
    fn altivec_memory_store_corpus_passes_against_executor() {
        let cases = cases();
        assert!(
            !cases.is_empty(),
            "AltiVec-memory store corpus must produce at least one case"
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
                "AltiVec-memory store corpus: {} failure(s) of {}:\n{detail}",
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
        for prefix in ["stvebx_", "stvehx_", "stvewx_", "stvxl_"] {
            assert!(
                labels.iter().any(|l| l.starts_with(prefix)),
                "corpus missing any '{prefix}' case"
            );
        }
    }
}
