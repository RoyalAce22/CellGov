use serde_json::{json, Value};

use super::*;

/// One `stop` vector whose components are all unsupported for `reason`.
fn set_with_reason(reason: &str) -> Value {
    let expected: serde_json::Map<String, Value> = [
        "end",
        "regs_hex",
        "local_store",
        "pc",
        "lslr",
        "fpscr",
        "stop",
        "interrupts_enabled",
        "srr0",
        "signals",
        "channels",
        "reservation",
        "effects",
        "main_memory",
        "peer",
        "ppu_results",
        "mfc_exceptions",
    ]
    .iter()
    .map(|name| {
        (
            name.to_string(),
            json!({"status": "unsupported", "reason": "not under test"}),
        )
    })
    .collect();
    let mut set = json!({
        "schema_version": 6,
        "unit": {"kind": "facility", "name": "command_queue"},
        "vectors": [{
            "name": "v",
            "provenance": {
                "kind": "documented_vector",
                "citation": "SPU-ISA p:238 s:10. Control Instructions",
                "vector_id": "v"
            },
            "words": [0],
            "initial_state": {"pc": 0},
            "step_limit": 1,
            "expected": expected
        }]
    });
    set["vectors"][0]["expected"]["pc"] = json!({"status": "undefined", "reason": reason});
    set
}

fn refusal(reason: &str) -> Option<&'static str> {
    match parse_reference_set_json(&set_with_reason(reason).to_string()) {
        Ok(_) => None,
        Err(SpuReferenceError::InVector { index: 0, source }) => match *source {
            SpuReferenceError::Invalid { field } => Some(field),
            other => panic!("{reason:?}: refused for another cause: {other:?}"),
        },
        Err(other) => panic!("{reason:?}: refused for another cause: {other:?}"),
    }
}

#[test]
fn a_reason_naming_a_tracker_issue_refuses_and_other_hashes_pass() {
    for reason in [
        "left to hardware capture, issue #1411",
        "#7 says so",
        "see #12, then the rule",
    ] {
        assert_eq!(refusal(reason), Some("expected.pc"), "{reason:?}");
    }
    for reason in [
        "the rule leaves it open (SPU-ISA p:180)",
        "a # alone, or #x, names no issue",
        "ends with #",
    ] {
        assert_eq!(refusal(reason), None, "{reason:?}");
    }
}
