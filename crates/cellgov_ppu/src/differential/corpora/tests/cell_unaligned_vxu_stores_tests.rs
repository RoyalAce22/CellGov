//! Cell-unaligned VXU store differential corpus, including stvlx+stvrx full-vector composition.

use super::super::super::{assert_case, execute_into_memory, run_corpus};
use super::*;

/// Run one store into `memory` with RA=BASE_ADDR, RB=`ea_offset`,
/// and VS holding `VS_PATTERN`; return the resulting memory image.
fn exec_store(raw: u32, vs_index: u8, ea_offset: u64, memory: Vec<u8>) -> Vec<u8> {
    let initial = state_with_ra_rb_vs(
        RA_IDX as usize,
        BASE_ADDR,
        RB_IDX as usize,
        ea_offset,
        vs_index as usize,
        VS_PATTERN,
    );
    execute_into_memory(raw, &initial, BASE_ADDR, memory)
}

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

// Re-runs what the batch test above covers; kept for per-case
// isolation and finer failure attribution.
#[test]
fn each_case_passes_through_assert_case() {
    for case in cases() {
        assert_case(&case);
    }
}

#[test]
fn corpus_covers_all_four_ops() {
    let cases = cases();
    let labels: Vec<&str> = cases.iter().map(|c| c.label.as_str()).collect();
    for prefix in ["stvlx_", "stvrx_", "stvlxl_", "stvrxl_"] {
        assert!(
            labels.iter().any(|l| l.starts_with(prefix)),
            "corpus missing any '{prefix}' case"
        );
    }
}

// Every corpus case expects initial == post registers, so a passing
// run_corpus also proves the executor clobbered no register.
#[test]
fn store_cases_expect_no_register_side_effects() {
    for case in cases() {
        assert_eq!(
            case.initial_state, case.expected_state,
            "'{}' expects a register change from a store",
            case.label
        );
    }
}

#[test]
fn stvlx_plus_stvrx_compose_into_full_unaligned_vector() {
    let vs_bytes = VS_PATTERN.to_be_bytes();

    // stvlx @ EA=BASE+5 writes VS[0..11] at [5..16]; stvrx @
    // EA=BASE+21 writes VS[11..16] at the aligned line below (16).
    // Both run into the SAME memory image.
    let mem = exec_store(
        xform(VS_IDX_STVLX, RA_IDX, RB_IDX, XO_STVLX),
        VS_IDX_STVLX,
        5,
        vec![0u8; 32],
    );
    let mem = exec_store(
        xform(VS_IDX_STVLX, RA_IDX, RB_IDX, XO_STVRX),
        VS_IDX_STVLX,
        5 + 16,
        mem,
    );

    // The whole 16-byte vector must be present at [5..21]...
    assert_eq!(
        &mem[5..21],
        &vs_bytes[..],
        "composed pair did not reproduce the full vector"
    );
    // ...and nothing outside [5..21] may be touched.
    assert!(mem[..5].iter().all(|&b| b == 0), "wrote below EA");
    assert!(mem[21..].iter().all(|&b| b == 0), "wrote past EA+16");
}

#[test]
fn last_variants_are_byte_identical_to_base() {
    for (xo_base, xo_last) in [(XO_STVLX, XO_STVLXL), (XO_STVRX, XO_STVRXL)] {
        for ea_offset in [0u64, 1, 5, 8, 15, 17, 31] {
            let mem_base = exec_store(
                xform(VS_IDX_STVLX, RA_IDX, RB_IDX, xo_base),
                VS_IDX_STVLX,
                ea_offset,
                vec![0u8; 48],
            );
            let mem_last = exec_store(
                xform(VS_IDX_STVLX, RA_IDX, RB_IDX, xo_last),
                VS_IDX_STVLX,
                ea_offset,
                vec![0u8; 48],
            );
            assert_eq!(
                mem_base, mem_last,
                "xo {xo_base} and {xo_last} diverged at offset {ea_offset}"
            );
        }
    }
}
