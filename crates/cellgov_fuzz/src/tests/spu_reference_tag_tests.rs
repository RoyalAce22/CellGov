//! A reference artifact's MFC_Cmd count is 0 to 16 free slots, and a
//! waiting tag-status update request is any or all.

use super::*;

// [CBE-Handbook p:528 s:19.4.3.2] the MFC SPU command queue has 16 entries.

const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/spu_reference_single/rotqbyi_12_v1.json"
));

fn with_command_count(side: &str, count: u32) -> serde_json::Value {
    let mut json: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture is JSON");
    let channels = serde_json::json!({
        "mfc_lsa": 0, "mfc_eah": 0, "mfc_eal": 0, "mfc_size": 0, "mfc_tag_id": 0,
        "tag_mask": 0, "tag_status": 0, "atomic_status": 0,
        "mfc_cmd_count": count,
        "tag_update": null, "tag_status_read": null, "atomic_status_ready": false, "in_mbox": [],
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
fn a_command_count_past_the_queue_depth_is_refused_on_either_side() {
    for (side, field) in [
        ("initial_state", "initial_state.channels.mfc_cmd_count"),
        ("expected", "expected.channels.mfc_cmd_count"),
    ] {
        let refused = parse_reference_json(&with_command_count(side, 17).to_string());
        assert!(
            matches!(refused, Err(SpuReferenceError::Invalid { field: f }) if f == field),
            "{side}: {refused:?}"
        );
    }
}

#[test]
fn a_full_and_an_empty_queue_load_as_their_counts() {
    for count in [0, 16] {
        let artifact =
            parse_reference_json(&with_command_count("initial_state", count).to_string())
                .expect("0 and 16 free slots are both valid");
        let replay = replay_reference(&artifact).expect("the artifact replays");
        assert_eq!(replay.initial.channels.cmd_queue_free, count);
    }
    let mut json = with_command_count("initial_state", 0);
    json["initial_state"]["channels"]
        .as_object_mut()
        .expect("an object")
        .remove("mfc_cmd_count");
    let artifact = parse_reference_json(&json.to_string()).expect("the count is optional");
    let replay = replay_reference(&artifact).expect("the artifact replays");
    assert_eq!(
        replay.initial.channels.cmd_queue_free, 16,
        "absent is a full queue free"
    );
}

/// [CBE-Handbook p:459 s:17.10] TS 00 updates at once and 11 is reserved, so only 01 and 10 leave a request waiting.
#[test]
fn a_waiting_tag_update_other_than_any_or_all_is_refused_on_either_side() {
    for (side, field) in [
        ("initial_state", "initial_state.channels.tag_update"),
        ("expected", "expected.channels.tag_update"),
    ] {
        for (ts, accepted) in [(0u32, false), (1, true), (2, true), (3, false)] {
            let mut json = with_command_count(side, 16);
            let channels = if side == "initial_state" {
                &mut json["initial_state"]["channels"]
            } else {
                &mut json["expected"]["channels"]["value"]
            };
            channels["tag_update"] = ts.into();
            let parsed = parse_reference_json(&json.to_string());
            if accepted {
                assert!(parsed.is_ok(), "{side} TS {ts}: {parsed:?}");
            } else {
                assert!(
                    matches!(parsed, Err(SpuReferenceError::Invalid { field: f }) if f == field),
                    "{side} TS {ts}: {parsed:?}"
                );
            }
        }
    }
}
