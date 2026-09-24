use std::time::Duration;

use cellgov_boot::prepare::StartupTimings;
use cellgov_boot::step_loop::{RunAnomalies, StepTiming};

use super::{anomaly_lines, frequency_block, startup_timing_lines, step_profile_lines};

#[test]
fn quiet_run_reports_no_anomaly() {
    assert!(anomaly_lines(&RunAnomalies::default()).is_empty());
}

#[test]
fn each_counter_gets_its_own_line() {
    let c = RunAnomalies {
        provisional_reads: 1,
        response_displacements: 2,
        tty_oob_dropped: 3,
        tty_bogus_fd: 4,
    };
    let lines = anomaly_lines(&c);
    assert_eq!(lines.len(), 4);
    assert!(lines[0].starts_with("provisional_reads: 1"));
    assert!(lines[1].starts_with("syscall_response_displacements: 2"));
    assert!(lines[2].starts_with("tty_oob_captures_dropped: 3"));
    assert!(lines[3].starts_with("tty_bogus_fd_calls: 4"));
}

#[test]
fn zero_loop_time_qualifies_every_percentage() {
    let lines = step_profile_lines(&StepTiming::default(), Duration::ZERO, 100);
    assert!(lines.iter().any(|l| l.contains("WARN: t_loop is zero")));
    assert!(lines.iter().any(|l| l.contains("steps/sec:     n/a")));
    assert!(!lines.iter().any(|l| l.contains("inf")));
}

#[test]
fn a_measured_loop_reports_a_rate() {
    let t = StepTiming {
        step_time: Duration::from_millis(400),
        commit_time: Duration::from_millis(200),
        coverage_time: Duration::from_millis(100),
    };
    let lines = step_profile_lines(&t, Duration::from_secs(1), 2000);
    assert!(!lines.iter().any(|l| l.contains("WARN")));
    assert!(lines
        .iter()
        .any(|l| l.contains("step (sched)") && l.contains("40.0%")));
    assert!(lines
        .iter()
        .any(|l| l.contains("other overhead") && l.contains("30.0%")));
    assert!(lines.iter().any(|l| l.contains("steps/sec:     2000")));
}

#[test]
fn buckets_over_the_loop_total_report_the_excess() {
    let t = StepTiming {
        step_time: Duration::from_secs(2),
        commit_time: Duration::ZERO,
        coverage_time: Duration::ZERO,
    };
    let lines = step_profile_lines(&t, Duration::from_secs(1), 1);
    assert!(lines
        .iter()
        .any(|l| l.contains("other overhead: WARN tracked buckets exceed")));
}

#[test]
fn the_startup_block_totals_its_stages_and_ends_blank() {
    let lines = startup_timing_lines(&StartupTimings {
        mem_alloc: Duration::from_millis(1),
        elf_load: Duration::from_millis(2),
        hle_bind: Duration::from_millis(3),
        prx_load: Duration::from_millis(4),
    });
    assert_eq!(lines.len(), 7);
    assert_eq!(lines[0], "startup timing:");
    assert!(lines[1..5].iter().all(|l| l.starts_with("  ")));
    assert!(lines[5].starts_with("  total startup:") && lines[5].ends_with("10ms"));
    assert!(lines[6].is_empty());
}

#[test]
fn an_empty_tally_reports_a_header_and_no_rows() {
    let rows: [(&str, u64); 0] = [];
    let lines = frequency_block("unit 0: instruction frequency", &rows, 40);
    assert_eq!(lines.len(), 2);
    assert!(lines[1].contains("total=0"));
}

#[test]
fn a_tally_reports_at_most_the_row_limit() {
    let rows: Vec<(String, u64)> = (0..10).map(|i| (format!("insn{i}"), 10)).collect();
    let lines = frequency_block("unit 0: instruction frequency", &rows, 3);
    assert_eq!(lines.len(), 5);
    assert!(lines[1].contains("top 3") && lines[1].contains("total=100"));
    assert!(lines[2].contains("10.00%") && lines[2].ends_with("insn0"));
}

#[test]
fn a_zero_total_tally_reports_no_percentage_of_nothing() {
    let rows = vec![("insn".to_string(), 0u64)];
    let lines = frequency_block("unit 0: instruction frequency", &rows, 40);
    assert!(lines[2].contains("0.00%"));
    assert!(!lines[2].contains("NaN"));
}
