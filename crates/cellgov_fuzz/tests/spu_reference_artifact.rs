//! Tests offline SPU vectors and independent-reference refusals.

use cellgov_fuzz::spu_reference::{
    compare_reference, parse_reference_json, replay_reference, SpuReferenceComponent,
    SpuReferenceError, SpuReferenceOmission,
};

const ROTATION: &str = include_str!("fixtures/spu_reference_single/rotqbyi_12_v1.json");
const DIRECTED_ROUNDING: &str =
    include_str!("fixtures/spu_reference/dfa_directed_rounding_v1.json");

/// [SPU-ISA p:197 s:9.2] slice 0 rounds by RN0 and slice 1 by RN1: 1 + 0.75 ulp toward zero is 1, and -(1 + 0.25 ulp) toward -inf is -(1 + 1 ulp), where round to nearest gives 1 + 1 ulp and -1.
/// [SPU-ISA p:200 s:9.3] the slice-0 INV the vector starts with stays set, and each inexact slice adds its INX.
#[test]
fn a_vector_under_directed_rounding_replays_from_its_initial_fpscr() {
    let artifact = parse_reference_json(DIRECTED_ROUNDING).expect("committed vector must parse");
    let replay = replay_reference(&artifact).expect("committed vector must execute");
    assert!(replay.comparison.is_match(), "{:?}", replay.comparison);
    for component in [
        SpuReferenceComponent::Registers,
        SpuReferenceComponent::Fpscr,
    ] {
        assert!(
            replay.comparison.compared.contains(&component),
            "{component:?}"
        );
    }
    assert_eq!(
        replay.initial.fpscr,
        0x0000_0700_0000_0400_0000_0000_0000_0000
    );
    assert_eq!(
        u128::from_be_bytes(replay.state.regs[3]),
        0x3ff0_0000_0000_0000_bff0_0000_0000_0001
    );
}

#[test]
fn a_malformed_or_undefined_fpscr_is_refused_on_either_side() {
    let json: serde_json::Value = serde_json::from_str(DIRECTED_ROUNDING).expect("valid JSON");
    // Bit 0 is not an FPSCR field; the others are short or not lowercase hex.
    let undefined = format!("8{}", "0".repeat(31));
    for value in [
        undefined.as_str(),
        "0700",
        "0000070000000C000000080000000000",
        &"g".repeat(32),
    ] {
        let mut initial = json.clone();
        initial["initial_state"]["fpscr"] = value.into();
        assert!(
            matches!(
                parse_reference_json(&initial.to_string()),
                Err(SpuReferenceError::Invalid {
                    field: "initial_state.fpscr"
                })
            ),
            "initial {value}"
        );
        let mut expected = json.clone();
        expected["expected"]["fpscr"]["value"] = value.into();
        assert!(
            matches!(
                parse_reference_json(&expected.to_string()),
                Err(SpuReferenceError::Invalid {
                    field: "expected.fpscr"
                })
            ),
            "expected {value}"
        );
    }
    let mut absent = json;
    absent["initial_state"]
        .as_object_mut()
        .expect("an object")
        .remove("fpscr");
    let artifact = parse_reference_json(&absent.to_string()).expect("the field is optional");
    let replay = replay_reference(&artifact).expect("replays");
    assert_eq!(replay.initial.fpscr, 0);
    assert!(replay
        .comparison
        .differences
        .contains(&SpuReferenceComponent::Fpscr));
}

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
    for found in [1, 2, 3, 4, 6] {
        // Version 1 predates the FPSCR axis, version 2 the initial FPSCR,
        // version 3 the inbound mailbox contents, version 4 the MFC_Cmd
        // count; version 6 is not written yet.
        json["schema_version"] = found.into();
        assert!(matches!(
            parse_reference_json(&json.to_string()),
            Err(SpuReferenceError::Version { found: f, supported: 5 }) if f == found
        ));
    }
    json["schema_version"] = 5.into();
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
fn a_stashed_mailbox_register_is_no_longer_a_channel_field() {
    // A mailbox read writes its register when it runs, so no channel
    // state holds a destination register between steps.
    let mut json: serde_json::Value = serde_json::from_str(ROTATION).expect("valid JSON");
    json["expected"]["channels"] = serde_json::json!({"status": "value", "value": {
        "mfc_lsa": 0, "mfc_eah": 0, "mfc_eal": 0, "mfc_size": 0,
        "mfc_tag_id": 0, "tag_mask": 0, "tag_status": 0, "atomic_status": 0,
        "pending_mbox_rt": 3
    }});
    assert!(matches!(
        parse_reference_json(&json.to_string()),
        Err(SpuReferenceError::Json(_))
    ));
}

#[test]
fn decode_refusals_keep_the_raw_word_and_program_counter() {
    let mut json: serde_json::Value = serde_json::from_str(ROTATION).expect("valid JSON");
    // Every CBE instruction decodes, so the first SPU instruction the
    // CBE does not provide stands in.
    let (word, mnemonic) = first_absent_on_cbe_row();
    json["words"] = serde_json::json!([word]);
    let artifact =
        parse_reference_json(&json.to_string()).expect("decoder robustness word must parse");
    let error = replay_reference(&artifact).expect_err("unsupported word must refuse");
    assert!(matches!(
        error,
        SpuReferenceError::Decode {
            pc: 0,
            source: cellgov_spu::instruction::SpuDecodeError::AbsentOnCbe {
                raw,
                mnemonic: refused,
            }
        } if raw == word && refused == mnemonic
    ));
}

/// The canonical word and mnemonic of the first SPU instruction the CBE
/// does not provide.
fn first_absent_on_cbe_row() -> (u32, &'static str) {
    cellgov_ps3_abi::hw::spu_isa::SPU_OPCODE_MAP
        .iter()
        .find(|row| !row.on_cbe)
        .map(|row| (row.canonical_word(), row.mnemonic))
        .expect("an SPU instruction absent on the CBE")
}
