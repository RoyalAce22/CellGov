//! AltiVec memory-store differential corpus runs clean against the executor.

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
    let labels: Vec<&str> = cases.iter().map(|c| c.label.as_str()).collect();
    for prefix in ["stvebx_", "stvehx_", "stvewx_", "stvxl_"] {
        assert!(
            labels.iter().any(|l| l.starts_with(prefix)),
            "corpus missing any '{prefix}' case"
        );
    }
}
