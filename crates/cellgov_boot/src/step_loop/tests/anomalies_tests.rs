use super::*;

#[test]
fn a_quiet_run_has_no_critical_anomaly() {
    assert!(!RunAnomalies::default().had_critical_anomaly());
}

#[test]
fn only_a_displaced_response_is_critical() {
    let displaced = RunAnomalies {
        response_displacements: 1,
        ..RunAnomalies::default()
    };
    assert!(displaced.had_critical_anomaly());
    let dropped = RunAnomalies {
        provisional_reads: 9,
        tty_oob_dropped: 9,
        tty_bogus_fd: 9,
        ..RunAnomalies::default()
    };
    assert!(!dropped.had_critical_anomaly());
}
