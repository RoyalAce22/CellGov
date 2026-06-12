//! Observation equality sensitivity and JSON round-trips, including legacy baselines without tty_log.

use super::*;
use crate::observation::event::ObservedEventKind;
use crate::test_support::sample_observation;

#[test]
fn observations_with_same_fields_are_equal() {
    let a = sample_observation();
    let b = sample_observation();
    assert_eq!(a, b);
}

#[test]
fn different_outcome_is_not_equal() {
    let a = sample_observation();
    let mut b = sample_observation();
    b.outcome = ObservedOutcome::Timeout;
    assert_ne!(a, b);
}

#[test]
fn different_memory_region_data_is_not_equal() {
    let a = sample_observation();
    let mut b = sample_observation();
    b.memory_regions[0].data = vec![0, 0, 0, 2];
    assert_ne!(a, b);
}

#[test]
fn different_event_sequence_is_not_equal() {
    let a = sample_observation();
    let mut b = sample_observation();
    b.events[1].kind = ObservedEventKind::UnitBlock;
    assert_ne!(a, b);
}

#[test]
fn missing_hashes_differs_from_present() {
    let a = sample_observation();
    let mut b = sample_observation();
    b.state_hashes = None;
    assert_ne!(a, b);
}

#[test]
fn rpcs3_style_observation_has_no_hashes_or_steps() {
    let obs = Observation {
        outcome: ObservedOutcome::Completed,
        memory_regions: vec![NamedMemoryRegion {
            name: "result".into(),
            addr: 0x10000,
            data: vec![0, 0, 0, 1],
        }],
        events: vec![],
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: "rpcs3".into(),
            steps: None,
        },
        tty_log: Vec::new(),
    };
    assert!(obs.state_hashes.is_none());
    assert!(obs.metadata.steps.is_none());
}

#[test]
fn observation_with_hashes_roundtrips() {
    let obs = sample_observation();
    let json = serde_json::to_string(&obs).expect("serialize");
    let loaded: Observation = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(obs, loaded);
}

#[test]
fn observation_without_hashes_roundtrips() {
    let obs = Observation {
        outcome: ObservedOutcome::Completed,
        memory_regions: vec![],
        events: vec![],
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: "rpcs3".into(),
            steps: None,
        },
        tty_log: Vec::new(),
    };
    let json = serde_json::to_string(&obs).expect("serialize");
    let loaded: Observation = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(obs, loaded);
}

#[test]
fn observation_without_tty_log_field_loads_with_empty_default() {
    let json = r#"{
        "outcome": "Completed",
        "memory_regions": [],
        "events": [],
        "state_hashes": null,
        "metadata": { "runner": "rpcs3-interpreter", "steps": null }
    }"#;
    let obs: Observation = serde_json::from_str(json).expect("legacy baseline must load");
    assert!(obs.tty_log.is_empty());
}

#[test]
fn tty_log_difference_breaks_observation_equality() {
    let a = sample_observation();
    let mut b = sample_observation();
    b.tty_log.push(b'!');
    assert_ne!(a, b);
}

#[test]
fn null_on_required_field_rejects() {
    let json = r#"{
        "outcome": "Completed",
        "memory_regions": null,
        "events": [],
        "state_hashes": null,
        "metadata": { "runner": "rpcs3", "steps": null }
    }"#;
    assert!(
        serde_json::from_str::<Observation>(json).is_err(),
        "null memory_regions must fail to deserialize, not default to empty"
    );
}
