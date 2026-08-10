//! `clear_observability` contract: instruments reset, undrained
//! diagnostics survive.

use crate::host::Lv2Host;

#[test]
fn clear_observability_keeps_undrained_invariant_breaks_for_the_trace_drain() {
    let mut host = Lv2Host::new();
    host.log_invariant_break(
        "test.commit_time_site",
        format_args!("pushed after the dispatch-time drain"),
    );
    host.clear_observability();
    assert_eq!(
        host.drain_pending_invariant_breaks().count(),
        1,
        "an undrained break must survive the instrument wipe or its \
         HostInvariantBreak trace record is silently dropped",
    );
}

#[test]
fn clear_observability_resets_instruments_to_the_boot_state() {
    let mut host = Lv2Host::new();
    host.log_invariant_break("test.site", format_args!("bump"));
    for _ in host.drain_pending_invariant_breaks() {}
    host.clear_observability();
    let obs = host.observability();
    assert_eq!(obs.invariant_break_count, 0);
    assert!(obs.invariant_break_sites.is_empty());
    assert!(obs.pending_invariant_breaks.is_empty());
}
