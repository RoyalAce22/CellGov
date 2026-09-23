//! Tests offline SPU vectors and independent-reference refusals.

use cellgov_fuzz::spu_reference::{
    compare_reference, parse_reference_json, replay_reference, SpuReferenceComponent,
    SpuReferenceError, SpuReferenceOmission,
};

const ROTATION: &str = include_str!("fixtures/spu_reference/rotqbyi_12_v1.json");

#[test]
fn documented_spu_vector_replays_without_operator_inputs() {
    let artifact = parse_reference_json(ROTATION).expect("committed vector must parse");
    let replay = replay_reference(&artifact).expect("committed vector must execute");
    assert!(replay.comparison.is_match(), "{:?}", replay.comparison);
    for component in [
        SpuReferenceComponent::Registers,
        SpuReferenceComponent::LocalStore,
        SpuReferenceComponent::ProgramCounter,
        SpuReferenceComponent::Outcome,
        SpuReferenceComponent::Effects,
        SpuReferenceComponent::FaultDiscard,
    ] {
        assert!(
            replay.comparison.compared.contains(&component),
            "{component:?}"
        );
    }
    assert_eq!(
        replay.state.regs[4],
        [12, 13, 14, 15, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
    );
    assert_eq!(
        replay
            .comparison
            .unrepresented
            .get(&SpuReferenceComponent::Channels),
        Some(&SpuReferenceOmission::Unsupported)
    );
    assert_eq!(
        replay
            .comparison
            .unrepresented
            .get(&SpuReferenceComponent::Reservation),
        Some(&SpuReferenceOmission::Unsupported)
    );
}

#[test]
fn a_shared_spu_executor_defect_escapes_replay_but_not_the_independent_vector() {
    let artifact = parse_reference_json(ROTATION).expect("committed vector must parse");
    let replay = replay_reference(&artifact).expect("committed vector must execute");
    let mut first = replay.state.clone();
    let mut second = replay.state.clone();
    first.regs[4][0] ^= 1;
    second.regs[4][0] ^= 1;
    assert_eq!(
        first, second,
        "internal identical replays cannot find this defect"
    );
    let comparison =
        compare_reference(&artifact.expected, &replay.initial, &first, &replay.outcome)
            .expect("same-source observation must compare");
    assert_eq!(
        comparison.differences,
        [SpuReferenceComponent::Registers].into()
    );
}

#[test]
fn unknown_schema_fields_and_versions_are_refused() {
    let mut json: serde_json::Value = serde_json::from_str(ROTATION).expect("valid JSON");
    json["schema_version"] = 2.into();
    assert!(matches!(
        parse_reference_json(&json.to_string()),
        Err(SpuReferenceError::Version {
            found: 2,
            supported: 1
        })
    ));
    json["schema_version"] = 1.into();
    json["expected"]["untracked_axis"] = true.into();
    assert!(matches!(
        parse_reference_json(&json.to_string()),
        Err(SpuReferenceError::Json(_))
    ));
}

#[test]
fn malformed_vectors_and_omission_reasons_cannot_be_silently_accepted() {
    for (path, value) in [
        ("hex", serde_json::json!("short")),
        ("reason", serde_json::json!("  ")),
    ] {
        let mut json: serde_json::Value = serde_json::from_str(ROTATION).expect("valid JSON");
        if path == "hex" {
            json["initial_state"]["regs_hex"]["2"] = value;
        } else {
            json["expected"]["channels"]["reason"] = value;
        }
        assert!(
            matches!(
                parse_reference_json(&json.to_string()),
                Err(SpuReferenceError::Invalid { .. })
            ),
            "{path}"
        );
    }
}

#[test]
fn hardware_capture_metadata_needs_a_digest_but_not_a_live_device() {
    let mut json: serde_json::Value = serde_json::from_str(ROTATION).expect("valid JSON");
    json["provenance"] = serde_json::json!({
        "kind": "hardware_capture", "capture_id": "test-only",
        "device": "not a claimed real capture", "environment": "synthetic parser test",
        "source_sha256": "not-a-digest"
    });
    assert!(matches!(
        parse_reference_json(&json.to_string()),
        Err(SpuReferenceError::Invalid {
            field: "provenance"
        })
    ));
    json["provenance"]["source_sha256"] = "a".repeat(64).into();
    assert!(parse_reference_json(&json.to_string()).is_ok());
}

#[test]
fn unrepresented_effect_payloads_require_an_explicit_exclusion() {
    let mut json: serde_json::Value = serde_json::from_str(ROTATION).expect("valid JSON");
    json["expected"]["effects"]["value"] = serde_json::json!(["untyped effect"]);
    assert!(matches!(
        parse_reference_json(&json.to_string()),
        Err(SpuReferenceError::Invalid {
            field: "expected.effects"
        })
    ));
}

#[test]
fn undefined_fields_remain_named_and_do_not_participate_in_comparison() {
    let mut json: serde_json::Value = serde_json::from_str(ROTATION).expect("valid JSON");
    json["expected"]["channels"] = serde_json::json!({"status": "undefined", "reason": "the source has no defined channel observation"});
    let artifact = parse_reference_json(&json.to_string()).expect("named limitation must parse");
    let replay = replay_reference(&artifact).expect("named limitation must replay");
    assert_eq!(
        replay
            .comparison
            .unrepresented
            .get(&SpuReferenceComponent::Channels),
        Some(&SpuReferenceOmission::Undefined)
    );
    assert!(!replay
        .comparison
        .compared
        .contains(&SpuReferenceComponent::Channels));
}

#[test]
fn sparse_numeric_keys_are_canonical_and_inside_the_architected_banks() {
    for (bank, key) in [
        ("regs_hex", "128"),
        ("regs_hex", "02"),
        ("local_store", "262144"),
    ] {
        let mut json: serde_json::Value = serde_json::from_str(ROTATION).expect("valid JSON");
        json["initial_state"][bank][key] = if bank == "regs_hex" {
            serde_json::json!("00000000000000000000000000000000")
        } else {
            serde_json::json!(7)
        };
        assert!(
            matches!(
                parse_reference_json(&json.to_string()),
                Err(SpuReferenceError::Invalid { .. })
            ),
            "{bank}.{key}"
        );
    }
}

#[test]
fn the_documented_vector_needs_an_official_source_key() {
    let mut json: serde_json::Value = serde_json::from_str(ROTATION).expect("valid JSON");
    json["provenance"]["citation"] = "uncited notebook".into();
    assert!(matches!(
        parse_reference_json(&json.to_string()),
        Err(SpuReferenceError::Invalid {
            field: "provenance"
        })
    ));
    for malformed in [
        "SPU-ISA p:not-a-page s:fiction",
        "SPU-ISA p:0 s:6",
        "SPU-ISA p:132 s:  ",
    ] {
        json["provenance"]["citation"] = malformed.into();
        assert!(
            matches!(
                parse_reference_json(&json.to_string()),
                Err(SpuReferenceError::Invalid {
                    field: "provenance"
                })
            ),
            "{malformed}"
        );
    }
}

#[test]
fn expected_channels_cannot_name_a_nonexistent_mailbox_register() {
    let mut json: serde_json::Value = serde_json::from_str(ROTATION).expect("valid JSON");
    json["expected"]["channels"] = serde_json::json!({"status": "value", "value": {
        "mfc_lsa": 0, "mfc_eah": 0, "mfc_eal": 0, "mfc_size": 0,
        "mfc_tag_id": 0, "tag_mask": 0, "tag_status": 0, "atomic_status": 0,
        "pending_mbox_rt": 128, "pending_get": null
    }});
    assert!(matches!(
        parse_reference_json(&json.to_string()),
        Err(SpuReferenceError::Invalid {
            field: "expected.channels.pending_mbox_rt"
        })
    ));
}

#[test]
fn decode_refusals_keep_the_raw_word_and_program_counter() {
    let mut json: serde_json::Value = serde_json::from_str(ROTATION).expect("valid JSON");
    json["words"] = serde_json::json!([u32::MAX]);
    let artifact =
        parse_reference_json(&json.to_string()).expect("decoder robustness word must parse");
    let error = replay_reference(&artifact).expect_err("unsupported word must refuse");
    assert!(matches!(
        error,
        SpuReferenceError::Decode {
            pc: 0,
            source: cellgov_spu::instruction::SpuDecodeError::Unsupported(u32::MAX)
        }
    ));
}
