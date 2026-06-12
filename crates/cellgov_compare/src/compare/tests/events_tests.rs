//! First-divergence localization in observed event sequences under exact and prefix matching.

use super::*;
use crate::observation::ObservedEventKind;
use crate::test_support::event;

#[test]
fn event_divergence_reports_first_differing_event() {
    let exp = vec![
        event(ObservedEventKind::MailboxSend, 0, 0),
        event(ObservedEventKind::UnitWake, 1, 1),
    ];
    let act = vec![
        event(ObservedEventKind::MailboxSend, 0, 0),
        event(ObservedEventKind::UnitBlock, 1, 1),
    ];
    let d = find_event_divergence(&exp, &act, false).expect("diverges");
    assert_eq!(d.index, 1);
    assert_eq!(d.expected.unwrap().kind, ObservedEventKind::UnitWake);
    assert_eq!(d.actual.unwrap().kind, ObservedEventKind::UnitBlock);
}

#[test]
fn event_unit_mismatch_is_divergence() {
    let exp = vec![event(ObservedEventKind::MailboxSend, 0, 0)];
    let act = vec![event(ObservedEventKind::MailboxSend, 1, 0)];
    let d = find_event_divergence(&exp, &act, false).expect("diverges");
    assert_eq!(d.index, 0);
    assert_eq!(d.expected.unwrap().unit, 0);
    assert_eq!(d.actual.unwrap().unit, 1);
}

#[test]
fn strict_mode_diverges_on_different_event_lengths() {
    let exp = vec![event(ObservedEventKind::MailboxSend, 0, 0)];
    let act = vec![
        event(ObservedEventKind::MailboxSend, 0, 0),
        event(ObservedEventKind::UnitWake, 1, 1),
    ];
    let d = find_event_divergence(&exp, &act, false).expect("diverges");
    assert_eq!(d.index, 1);
    assert!(d.expected.is_none());
    assert!(d.actual.is_some());
}

#[test]
fn prefix_mode_matches_when_shorter_prefix_agrees() {
    let exp = vec![
        event(ObservedEventKind::MailboxSend, 0, 0),
        event(ObservedEventKind::UnitWake, 1, 1),
    ];
    let act = vec![
        event(ObservedEventKind::MailboxSend, 0, 0),
        event(ObservedEventKind::UnitWake, 1, 1),
        event(ObservedEventKind::MailboxReceive, 1, 2),
    ];
    assert!(find_event_divergence(&exp, &act, true).is_none());
}

#[test]
fn prefix_mode_diverges_when_prefix_differs() {
    let exp = vec![event(ObservedEventKind::MailboxSend, 0, 0)];
    let act = vec![event(ObservedEventKind::UnitBlock, 0, 0)];
    assert!(find_event_divergence(&exp, &act, true).is_some());
}
