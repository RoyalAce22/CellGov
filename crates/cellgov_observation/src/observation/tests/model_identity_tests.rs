//! Where the identity triple sits in a serialized observation, and
//! what an observation written before versioning reads back as.

use super::*;
use crate::test_support::sample_observation;

#[test]
fn the_identity_sits_beside_the_other_fields_not_under_a_key_of_its_own() {
    let json = serde_json::to_string(&sample_observation()).expect("serialize");
    let v: serde_json::Value = serde_json::from_str(&json).expect("parse back");
    assert!(v.get("identity").is_none(), "{json}");
    assert_eq!(v["firmware"]["version"], "4.91");
    assert_eq!(v["game"]["title_id"], "NPAA00001");
}

#[test]
fn an_unidentified_observation_writes_no_identity_keys() {
    let mut obs = sample_observation();
    obs.identity = crate::identity::RunIdentity::default();
    let json = serde_json::to_string(&obs).expect("serialize");
    assert!(!json.contains("\"firmware\""), "{json}");
    assert!(!json.contains("\"game\""), "{json}");
}

#[test]
fn an_observation_written_before_versioning_reads_as_unidentified() {
    let json = r#"{
        "outcome": "Completed",
        "memory_regions": [],
        "events": [],
        "state_hashes": null,
        "metadata": { "runner": "cellgov-boot", "steps": null }
    }"#;
    let obs: Observation = serde_json::from_str(json).expect("legacy baseline must load");
    assert!(obs.identity.is_empty());
}

/// `#[serde(flatten)]` routes unknown keys through serde's buffered
/// representation. The byte-bearing fields keep their own decoders, so
/// a payload larger than any buffer round-trips.
#[test]
fn a_large_byte_payload_round_trips_beside_the_flattened_identity() {
    let mut obs = sample_observation();
    obs.memory_regions[0].data = (0..=255u8).cycle().take(64 * 1024).collect();
    obs.tty_log = vec![b'x'; 8 * 1024];
    let json = serde_json::to_string(&obs).expect("serialize");
    let loaded: Observation = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(obs, loaded);
}
