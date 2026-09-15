//! The identity triple on a `boot_history.jsonl` line.

use super::*;
use crate::test_support::identity;

fn witnesses() -> BTreeMap<String, u64> {
    BTreeMap::from([("ldarx".to_string(), 1)])
}

fn entry(previous: Option<&BootHistoryEntry>, id: RunIdentity) -> Option<BootHistoryEntry> {
    BootHistoryEntry::new_if_changed(previous, 100, "ProcessExit", witnesses(), id)
}

#[test]
fn a_line_carries_the_triple_it_was_measured_under() {
    let id = identity("4.91", "NPAA00001", "base");
    let e = entry(None, id.clone()).expect("a first recording is always a move");
    let line = render_line(&e).expect("render");
    let value: serde_json::Value = serde_json::from_str(line.trim_end()).expect("valid json");
    assert_eq!(value["firmware"]["version"], "4.91");
    assert_eq!(value["game"]["version"], "base");
    assert_eq!(parse(&line).expect("parse"), vec![e]);
    assert_eq!(
        parse(&line).expect("parse")[0].identity,
        id,
        "the identity triple survives the round trip"
    );
}

#[test]
fn a_pre_versioning_line_writes_no_identity_keys() {
    let e = entry(None, RunIdentity::default()).expect("a first recording is always a move");
    let line = render_line(&e).expect("render");
    assert!(!line.contains("firmware"), "{line}");
    assert!(!line.contains("\"game\""), "{line}");
}

#[test]
fn a_pre_versioning_line_parses_with_an_empty_identity() {
    let line =
        "{\"steps\":1,\"outcome\":\"ProcessExit\",\"witnesses\":{},\"changed\":[\"steps\"]}\n";
    let entries = parse(line).expect("parse");
    assert!(entries[0].identity.is_empty());
    assert_eq!(
        render_line(&entries[0]).expect("render"),
        line,
        "re-recording an unchanged line reproduces it byte for byte"
    );
}

#[test]
fn a_different_triple_is_a_move_on_its_own() {
    let first = entry(None, identity("4.91", "NPAA00001", "base")).expect("first");
    let second = entry(Some(&first), identity("4.93", "NPAA00001", "base"))
        .expect("the firmware changed, so the measurement is against a different identity triple");
    assert_eq!(second.changed, vec!["identity".to_string()]);
}

#[test]
fn the_same_triple_twice_is_not_a_move() {
    let id = identity("4.91", "NPAA00001", "base");
    let first = entry(None, id.clone()).expect("first");
    assert!(entry(Some(&first), id).is_none());
}

#[test]
fn a_first_triple_over_a_pre_versioning_line_is_recorded_as_such() {
    let first = entry(None, RunIdentity::default()).expect("first");
    let second = entry(Some(&first), identity("4.91", "NPAA00001", "base"))
        .expect("the history gained an axis it did not have");
    assert_eq!(
        second.changed,
        vec!["identity (first recorded)".to_string()]
    );
}

#[test]
fn two_pre_versioning_lines_do_not_move_the_identity() {
    let first = entry(None, RunIdentity::default()).expect("first");
    assert!(
        entry(Some(&first), RunIdentity::default()).is_none(),
        "neither line names an identity triple, so there is nothing to have moved"
    );
}

#[test]
fn a_triple_recorded_after_a_pre_versioning_line_still_moves_later() {
    let first = entry(None, RunIdentity::default()).expect("first");
    let second =
        entry(Some(&first), identity("4.91", "NPAA00001", "base")).expect("first recorded");
    let third = entry(Some(&second), identity("4.93", "NPAA00001", "base"))
        .expect("the axis is live once a line names it");
    assert_eq!(third.changed, vec!["identity".to_string()]);
}

#[test]
fn naming_a_half_that_was_not_named_before_is_a_move() {
    let full = identity("4.91", "NPAA00001", "base");
    let fw_only = RunIdentity {
        firmware: full.firmware.clone(),
        game: None,
        overrides: Default::default(),
    };
    let first = entry(None, fw_only).expect("first");
    let second = entry(Some(&first), full)
        .expect("the run named a store entry where the previous one named none");
    assert_eq!(second.changed, vec!["identity".to_string()]);
}

#[test]
fn an_identity_move_is_named_alongside_the_witnesses_that_moved() {
    let first = entry(None, identity("4.91", "NPAA00001", "base")).expect("first");
    let second = BootHistoryEntry::new_if_changed(
        Some(&first),
        100,
        "ProcessExit",
        BTreeMap::from([("ldarx".to_string(), 2)]),
        identity("4.93", "NPAA00001", "base"),
    )
    .expect("a witness and the identity triple both moved");
    assert_eq!(
        second.changed,
        vec!["identity".to_string(), "ldarx".to_string()]
    );
}

#[test]
fn losing_a_triple_is_a_move() {
    let first = entry(None, identity("4.91", "NPAA00001", "base")).expect("first");
    let second = entry(Some(&first), RunIdentity::default())
        .expect("a line that no longer names its firmware is not the same measurement");
    assert_eq!(
        second.changed,
        vec!["identity (no longer recorded)".to_string()]
    );
}
