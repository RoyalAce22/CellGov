//! `log_invariant_break` keeps the first break's message and counts every break.

use crate::host::Lv2Host;

#[test]
fn the_reported_line_names_the_first_site_its_details_and_the_total() {
    let mut host = Lv2Host::new();
    assert_eq!(
        host.observability().first_invariant_break_line(),
        None,
        "a run that broke no invariant gives the driver no line to report"
    );

    host.log_invariant_break("sync.wake", format_args!("thread 7 not in table"));
    assert_eq!(
        host.observability().first_invariant_break_line().as_deref(),
        Some("lv2 host invariant break at sync.wake: thread 7 not in table (the first of 1)")
    );

    host.log_invariant_break("prx.load", format_args!("second"));
    assert_eq!(
        host.observability().first_invariant_break_line().as_deref(),
        Some("lv2 host invariant break at sync.wake: thread 7 not in table (the first of 2)"),
        "the line keeps the first break's site and details, and counts every break"
    );
}

#[test]
fn a_later_break_does_not_replace_the_first_message() {
    let mut host = Lv2Host::new();
    host.log_invariant_break("test.first", format_args!("one"));
    host.log_invariant_break("test.second", format_args!("two"));
    let obs = host.observability();
    assert_eq!(
        obs.first_invariant_break.as_deref(),
        Some("test.first: one")
    );
    assert_eq!(obs.invariant_break_count, 2);
    assert_eq!(host.invariant_break_site_count("test.second"), 1);
}

#[test]
fn clear_observability_rearms_the_first_message_with_the_count() {
    let mut host = Lv2Host::new();
    host.log_invariant_break("test.before", format_args!("one"));
    host.clear_observability();
    assert_eq!(host.observability().first_invariant_break, None);
    host.log_invariant_break("test.after", format_args!("two"));
    let obs = host.observability();
    assert_eq!(
        obs.first_invariant_break.as_deref(),
        Some("test.after: two")
    );
    assert_eq!(obs.invariant_break_count, 1);
    assert_eq!(host.drain_pending_invariant_breaks().count(), 2);
}

#[test]
fn the_reported_line_counts_only_breaks_since_the_last_observability_clear() {
    let mut host = Lv2Host::new();
    host.log_invariant_break("test.before", format_args!("one"));
    host.clear_observability();
    assert_eq!(
        host.observability().first_invariant_break_line(),
        None,
        "the clear takes the message, so there is no line left to report"
    );

    host.log_invariant_break("test.after", format_args!("two"));
    assert_eq!(
        host.observability().first_invariant_break_line().as_deref(),
        Some("lv2 host invariant break at test.after: two (the first of 1)"),
        "the clear resets the message and the count together; a count that \
         survived it would name this break as the first of a wider total"
    );
}
