//! The identity triple's place in the `boot_summary.json` wire shape.

use super::*;
use crate::test_support::identity;

fn summary() -> BootSummary {
    BootSummary::new(
        CheckpointKind::ProcessExit,
        BootOutcome::ProcessExit,
        1234,
        Budget::new(64),
    )
    .expect("consistent checkpoint / outcome")
}

#[test]
fn a_summary_with_no_identity_writes_no_identity_keys() {
    let json = serde_json::to_string(&summary()).expect("serialize");
    assert!(!json.contains("firmware"), "{json}");
    assert!(!json.contains("game"), "{json}");
}

#[test]
fn the_triple_lands_at_the_top_level_and_reads_back() {
    let mut s = summary();
    s.identity = identity("4.91", "NPAA00001", "update:02.51");
    let json = serde_json::to_string_pretty(&s).expect("serialize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert_eq!(value["firmware"]["version"], "4.91");
    assert_eq!(value["game"]["title_id"], "NPAA00001");
    assert_eq!(value["game"]["version"], "update:02.51");

    let back: BootSummary = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, s);
}

#[test]
fn a_pre_versioning_summary_still_loads() {
    let text = r#"{
        "checkpoint": {"kind": "process_exit"},
        "outcome": "ProcessExit",
        "steps": 10,
        "budget": 64,
        "host_invariant_breaks": 0
    }"#;
    let back: BootSummary = serde_json::from_str(text).expect("deserialize");
    assert!(back.identity.is_empty());
}

#[test]
fn an_identity_key_the_schema_does_not_own_is_refused() {
    let text = r#"{
        "checkpoint": {"kind": "process_exit"},
        "outcome": "ProcessExit",
        "steps": 10,
        "budget": 64,
        "firmware": {"version": "4.91", "image_version": "0x1", "pup_sha256": "ab", "extra": 1}
    }"#;
    assert!(serde_json::from_str::<BootSummary>(text).is_err());
}

#[test]
fn a_summary_with_no_identity_keeps_the_pre_versioning_wire_shape() {
    // `serde(flatten)` turns the whole struct into a serialized map;
    // this pins the rest of the wire shape against that.
    assert_eq!(
        serde_json::to_string(&summary()).expect("serialize"),
        r#"{"checkpoint":{"kind":"process_exit"},"outcome":"ProcessExit","steps":1234,"budget":64,"host_invariant_breaks":0}"#
    );
}

#[test]
fn a_pre_versioning_summary_reserializes_unchanged() {
    let text = r#"{"checkpoint":{"kind":"process_exit"},"outcome":"ProcessExit","steps":1234,"budget":64,"host_invariant_breaks":0}"#;
    let back: BootSummary = serde_json::from_str(text).expect("deserialize");
    assert_eq!(serde_json::to_string(&back).expect("serialize"), text);
}

#[test]
fn a_game_key_the_schema_does_not_own_is_refused() {
    let text = r#"{
        "checkpoint": {"kind": "process_exit"},
        "outcome": "ProcessExit",
        "steps": 10,
        "budget": 64,
        "game": {"title_id": "NPAA00001", "version": "base", "app_ver": "02.00", "extra": 1}
    }"#;
    assert!(serde_json::from_str::<BootSummary>(text).is_err());
}

#[test]
fn a_key_no_half_of_the_schema_owns_is_refused() {
    let text = r#"{
        "checkpoint": {"kind": "process_exit"},
        "outcome": "ProcessExit",
        "steps": 10,
        "budget": 64,
        "disc": {"version": "1.00"}
    }"#;
    assert!(
        serde_json::from_str::<BootSummary>(text).is_err(),
        "the shadow denies unknown fields, so a flattened key it does not name cannot slip past"
    );
}

#[test]
fn a_half_identity_reads_back_as_one_half() {
    let text = r#"{
        "checkpoint": {"kind": "process_exit"},
        "outcome": "ProcessExit",
        "steps": 10,
        "budget": 64,
        "game": {"title_id": "NPAA00001", "version": "base", "app_ver": "02.00"}
    }"#;
    let back: BootSummary = serde_json::from_str(text).expect("deserialize");
    assert!(back.identity.firmware.is_none());
    assert_eq!(
        back.identity.game.map(|g| g.title_id).as_deref(),
        Some("NPAA00001")
    );
}
