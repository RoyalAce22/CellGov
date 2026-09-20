//! AltiVec memory-store differential vector runs clean against the executor.

use super::super::super::{assert_case, run_vectors};
use super::*;

#[test]
fn altivec_memory_store_vectors_pass_against_executor() {
    let cases = cases();
    assert!(
        !cases.is_empty(),
        "AltiVec-memory store vectors must produce at least one case"
    );
    let report = run_vectors(&cases);
    if !report.is_clean() {
        let detail = report
            .failed
            .iter()
            .map(|(label, outcome)| format!("  '{label}': {outcome:?}"))
            .collect::<Vec<_>>()
            .join("\n");
        panic!(
            "AltiVec-memory store vectors: {} failure(s) of {}:\n{detail}",
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
fn vectors_cover_all_four_ops() {
    let cases = cases();
    let labels: Vec<&str> = cases.iter().map(|c| c.label.as_str()).collect();
    for prefix in ["stvebx_", "stvehx_", "stvewx_", "stvxl_"] {
        assert!(
            labels.iter().any(|l| l.starts_with(prefix)),
            "vectors missing any '{prefix}' case"
        );
    }
}
