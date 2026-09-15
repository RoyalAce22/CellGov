//! `log_invariant_break` keeps the first break's message and counts every break.

use crate::host::Lv2Host;

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
