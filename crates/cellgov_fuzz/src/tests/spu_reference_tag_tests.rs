//! A reference artifact's parked get names a tag group, 0 to 31.

use super::*;

// [CBEA p:115 s:9.1.3 MFC Command Tag Identification Channel] the identification tag is any value between x'0' and x'1F'.

const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/spu_reference/rotqbyi_12_v1.json"
));

fn with_parked_get_tag(side: &str, tag: u8) -> serde_json::Value {
    let mut json: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture is JSON");
    let channels = serde_json::json!({
        "mfc_lsa": 0, "mfc_eah": 0, "mfc_eal": 0, "mfc_size": 0, "mfc_tag_id": 0,
        "tag_mask": 0, "tag_status": 0, "atomic_status": 0,
        "pending_mbox_rt": null, "pending_get": [0, 0, 0, tag],
        "tag_update_pending": false, "atomic_status_ready": false, "in_mbox_count": 0,
        "out_mbox": null
    });
    if side == "initial_state" {
        json["initial_state"]["channels"] = channels;
    } else {
        json["expected"]["channels"] = serde_json::json!({"status": "value", "value": channels});
    }
    json
}

#[test]
fn a_parked_get_tag_past_31_is_refused_on_either_side() {
    for (side, field) in [
        ("initial_state", "initial_state.channels.pending_get"),
        ("expected", "expected.channels.pending_get"),
    ] {
        let refused = parse_reference_json(&with_parked_get_tag(side, 32).to_string());
        assert!(
            matches!(refused, Err(SpuReferenceError::Invalid { field: f }) if f == field),
            "{side}: {refused:?}"
        );
    }
}

#[test]
fn a_parked_get_tag_of_31_parses_and_loads_as_that_group() {
    let artifact = parse_reference_json(&with_parked_get_tag("initial_state", 31).to_string())
        .expect("tag 31 is the last group");
    let replay = replay_reference(&artifact).expect("the artifact replays");
    let (_, _, _, tag) = replay
        .initial
        .channels
        .pending_get
        .expect("the parked get loads");
    assert_eq!(tag.status_bit(), 1 << 31);
}
