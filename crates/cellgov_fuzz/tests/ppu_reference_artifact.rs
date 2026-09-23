//! Tests committed PPU reference artifacts and their offline replay.

use cellgov_fuzz::ppu_paths::first_path_divergence;
use cellgov_fuzz::ppu_reference::{
    compare_reference, parse_reference_json, replay_reference, PpuReferenceComponent,
    PpuReferenceError, PpuReferenceFieldStatus,
};

const LI_REFERENCE: &str = include_str!("fixtures/ppu_reference/li_r3_7_v1.json");

#[test]
fn a_committed_documented_vector_replays_offline_on_every_path() {
    let artifact = parse_reference_json(LI_REFERENCE).expect("committed reference must parse");

    let replay = replay_reference(&artifact).expect("committed reference must replay");

    assert!(replay.internal_divergence.is_none(), "{replay:#?}");
    assert_eq!(replay.comparisons.len(), 4);
    assert!(replay
        .comparisons
        .iter()
        .all(|comparison| comparison.is_match()));
    assert!(replay.comparisons.iter().all(|comparison| comparison
        .compared
        .contains(&PpuReferenceComponent::StateGpr)));
}

#[test]
fn unsupported_and_undefined_fields_remain_named_in_the_result() {
    let artifact = parse_reference_json(LI_REFERENCE).expect("committed reference must parse");

    let replay = replay_reference(&artifact).expect("committed reference must replay");
    let omitted = &replay.comparisons[0].unrepresented;

    assert!(omitted.iter().any(|field| {
        field.field == PpuReferenceComponent::StateFpr
            && field.status == PpuReferenceFieldStatus::Unsupported
    }));
    assert!(omitted.iter().any(|field| {
        field.field == PpuReferenceComponent::StateTb
            && field.status == PpuReferenceFieldStatus::Undefined
    }));
    assert!(!replay.comparisons[0]
        .compared
        .contains(&PpuReferenceComponent::StateFpr));
    assert!(!replay.comparisons[0]
        .compared
        .contains(&PpuReferenceComponent::StateTb));
}

#[test]
fn a_common_mode_internal_defect_still_fails_the_independent_reference() {
    let artifact = parse_reference_json(LI_REFERENCE).expect("committed reference must parse");
    let replay = replay_reference(&artifact).expect("committed reference must replay");
    let mut defective_runs = replay.runs;
    for run in &mut defective_runs {
        run.observation.state.gpr[3] = 8;
    }

    assert!(first_path_divergence(&defective_runs).is_none());
    for run in &defective_runs {
        let comparison = compare_reference(&artifact.expected, run);
        assert_eq!(comparison.differences.len(), 1);
        assert_eq!(
            comparison.differences[0].field,
            PpuReferenceComponent::StateGpr
        );
    }
}

#[test]
fn schema_drift_is_a_typed_refusal() {
    let mut json: serde_json::Value =
        serde_json::from_str(LI_REFERENCE).expect("committed reference must be JSON");
    json["schema_version"] = 2.into();

    let error = parse_reference_json(&json.to_string()).expect_err("version two must be refused");

    assert!(matches!(
        error,
        PpuReferenceError::Version {
            found: 2,
            supported: 1
        }
    ));
}

#[test]
fn unknown_fields_are_not_silently_discarded() {
    let mut json: serde_json::Value =
        serde_json::from_str(LI_REFERENCE).expect("committed reference must be JSON");
    json["expected"]["silent_new_field"] = true.into();

    let error = parse_reference_json(&json.to_string()).expect_err("unknown field must be refused");

    assert!(matches!(error, PpuReferenceError::Json(_)));
}

#[test]
fn hardware_capture_provenance_requires_a_sha256_digest() {
    let mut json: serde_json::Value =
        serde_json::from_str(LI_REFERENCE).expect("committed reference must be JSON");
    json["provenance"] = serde_json::json!({
        "kind": "hardware_capture",
        "capture_id": "operator-run-1",
        "device": "retail PS3",
        "environment": "operator supplied",
        "source_sha256": "not-a-digest"
    });

    let error = parse_reference_json(&json.to_string()).expect_err("bad digest must be refused");

    assert!(matches!(error, PpuReferenceError::CaptureDigest));
}

#[test]
fn valid_hardware_capture_provenance_is_replayable_repository_data() {
    let mut json: serde_json::Value =
        serde_json::from_str(LI_REFERENCE).expect("committed reference must be JSON");
    json["provenance"] = serde_json::json!({
        "kind": "hardware_capture",
        "capture_id": "operator-run-1",
        "device": "retail PS3",
        "environment": "operator supplied",
        "source_sha256": "0000000000000000000000000000000000000000000000000000000000000000"
    });

    let artifact = parse_reference_json(&json.to_string()).expect("valid capture must parse");
    let replay = replay_reference(&artifact).expect("valid capture must replay offline");

    assert!(replay
        .comparisons
        .iter()
        .all(|comparison| comparison.is_match()));
}

#[test]
fn documented_vectors_require_an_official_source_citation() {
    let mut json: serde_json::Value =
        serde_json::from_str(LI_REFERENCE).expect("committed reference must be JSON");
    json["provenance"]["citation"] = "informal note".into();

    let error = parse_reference_json(&json.to_string()).expect_err("informal source must fail");

    assert!(matches!(error, PpuReferenceError::Citation { .. }));
}

#[test]
fn omission_reasons_cannot_be_blank() {
    let mut json: serde_json::Value =
        serde_json::from_str(LI_REFERENCE).expect("committed reference must be JSON");
    json["expected"]["state"]["fpr"]["reason"] = "  ".into();

    let error = parse_reference_json(&json.to_string()).expect_err("blank reason must fail");

    assert!(matches!(
        error,
        PpuReferenceError::EmptyReason {
            field: "state.fpr",
            status: "unsupported"
        }
    ));
}

#[test]
fn vector_register_overrides_require_fixed_width_hex() {
    let mut json: serde_json::Value =
        serde_json::from_str(LI_REFERENCE).expect("committed reference must be JSON");
    json["initial_state"]["vr_hex"] = serde_json::json!({"2": "abc"});

    let error = parse_reference_json(&json.to_string()).expect_err("short vector must fail");

    assert!(matches!(error, PpuReferenceError::VectorValue { index: 2 }));
}

#[test]
fn unnormalized_effect_payloads_must_be_marked_unsupported() {
    let mut json: serde_json::Value =
        serde_json::from_str(LI_REFERENCE).expect("committed reference must be JSON");
    json["expected"]["committed_effects"] = serde_json::json!({
        "status": "value",
        "value": ["implementation debug text"]
    });

    let error = parse_reference_json(&json.to_string()).expect_err("string effect must fail");

    assert!(matches!(
        error,
        PpuReferenceError::UnsupportedValue {
            field: "committed_effects"
        }
    ));
}
