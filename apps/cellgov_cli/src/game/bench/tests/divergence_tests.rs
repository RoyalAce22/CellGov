use std::time::Duration;

use super::super::test_fixtures::{bench_manifest, bench_options, set_of, test_cell};
use super::*;

#[test]
fn a_long_boot_names_the_localization_commands_instead_of_running_them() {
    let title = bench_manifest(None);
    let cell = test_cell();
    let opts = bench_options(&title, Some(&cell), &[]);
    let mut runs = set_of(&[Duration::from_millis(100)]);
    runs[0].steps = LOCALIZE_MAX_STEPS + 1;
    let lines = locate_divergence(opts, &runs).expect("localization does not interrupt");
    assert!(lines[0].contains("not run automatically"), "got {lines:?}");
    assert!(
        lines.iter().any(|l| l.contains("--save-state-trace"))
            && lines.iter().any(|l| l.contains("diff diverge")),
        "the report must name every command the operator has to run: {lines:?}"
    );
}

#[test]
fn the_localization_cap_reads_the_longest_run_not_the_first() {
    let title = bench_manifest(None);
    let cell = test_cell();
    let opts = bench_options(&title, Some(&cell), &[]);
    let mut runs = set_of(&[Duration::from_millis(100); 2]);
    runs[0].steps = 10;
    runs[1].steps = LOCALIZE_MAX_STEPS + 1;
    let lines = locate_divergence(opts, &runs).expect("localization does not interrupt");
    assert!(lines[0].contains("not run automatically"), "got {lines:?}");
    assert!(
        lines[0].contains(&(LOCALIZE_MAX_STEPS + 1).to_string()),
        "the report must name the run that sets the cost: {lines:?}"
    );
}

#[test]
fn a_corrupt_trace_report_names_the_side_and_its_decode_failure() {
    let line = format_diverge(&cellgov_compare::DivergeReport::CorruptTrace {
        common_count: 12,
        a_error: None,
        b_error: Some(cellgov_compare::TraceDecodeError {
            index: 4,
            offset: 96,
            source: cellgov_trace::DecodeError::UnknownTag(0xee),
        }),
    });
    assert!(line.contains("a: ok"), "got {line}");
    assert!(
        line.contains("record 4") && line.contains("unknown record tag 0xee"),
        "got {line}"
    );
}

#[test]
fn a_failing_traced_re_run_reports_its_stderr_tail_in_order() {
    let stderr: String = (0..20).map(|i| format!("line {i}\n")).collect();
    let tail = stderr_tail(stderr.as_bytes());
    assert_eq!(tail.len(), 8);
    assert_eq!(tail.first().map(String::as_str), Some("  line 12"));
    assert_eq!(tail.last().map(String::as_str), Some("  line 19"));
}
