//! Byte-reverse load/store differential corpus runs clean against the executor.

use super::super::super::{assert_case, run_corpus};
use super::*;

#[test]
fn byte_reverse_corpus_passes_against_executor() {
    let cases = cases();
    assert!(
        !cases.is_empty(),
        "byte-reverse corpus must produce at least one case"
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
            "byte-reverse corpus: {} failure(s) of {}:\n{detail}",
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
