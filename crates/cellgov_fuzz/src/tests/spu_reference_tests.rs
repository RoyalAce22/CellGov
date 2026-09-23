use super::*;

use cellgov_effects::Effect;
use cellgov_exec::YieldReason;
use cellgov_spu::exec::SpuFault;
use cellgov_spu::instruction::SpuDecodeError;
use cellgov_sync::ReservedLine;

const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/spu_reference/rotqbyi_12_v1.json"
));

const ALL_COMPONENTS: [SpuReferenceComponent; 8] = [
    SpuReferenceComponent::Registers,
    SpuReferenceComponent::LocalStore,
    SpuReferenceComponent::ProgramCounter,
    SpuReferenceComponent::Channels,
    SpuReferenceComponent::Reservation,
    SpuReferenceComponent::Outcome,
    SpuReferenceComponent::Effects,
    SpuReferenceComponent::FaultDiscard,
];

const SEQUENTIAL: &str = "000102030405060708090a0b0c0d0e0f";
const ROTATED_12: &str = "0c0d0e0f000102030405060708090a0b";
// [SPU-ISA p:238 s:10. Control Instructions] stop: opcode 0x000 with a zero stop-and-signal type.
const STOP_WORD: u32 = 0;
// [SPU-ISA p:52 s:4. Constant-Formation Instructions] il RI16-form: opcode 0x081, I16 = 1, RT = 3.
const IL_R3_1: u32 = 0x4080_0083;
// [SPU-ISA p:249 s:11. Channel Instructions] rchcnt RR-form: opcode 0x00F, CA = 127, RT = 3.
const RCHCNT_R3_CH127: u32 = 0x01ff_c003;

type Mutation = fn(&mut SpuReferenceExpected, &mut SpuObservableSnapshot, &mut SpuStepOutcome);

fn fixture() -> serde_json::Value {
    serde_json::from_str(FIXTURE).expect("fixture is JSON")
}

fn parse(json: &serde_json::Value) -> Result<SpuReferenceArtifact, SpuReferenceError> {
    parse_reference_json(&json.to_string())
}

fn artifact() -> SpuReferenceArtifact {
    parse_reference_json(FIXTURE).expect("fixture parses")
}

fn value<T>(value: T) -> ReferenceField<T> {
    ReferenceField::Value { value }
}

fn snapshot() -> SpuObservableSnapshot {
    SpuObservableSnapshot::capture(&SpuState::new())
}

fn expectation_of(
    observed: &SpuObservableSnapshot,
    outcome: &SpuStepOutcome,
) -> SpuReferenceExpected {
    SpuReferenceExpected {
        regs_hex: value(BTreeMap::new()),
        local_store: value(BTreeMap::new()),
        pc: value(observed.pc),
        channels: value(SpuReferenceChannels::from(&observed.channels)),
        reservation: value(observed.reservation.map(|line| line.addr())),
        outcome: value(SpuReferenceOutcome::from(outcome)),
        effects: value(Vec::new()),
        fault_discarded: value(matches!(outcome, SpuStepOutcome::Fault(_))),
    }
}

fn trace_marker() -> Effect {
    Effect::TraceMarker {
        marker: 1,
        source: UnitId::new(0),
    }
}

fn single_difference(component: SpuReferenceComponent, mutate: Mutation) {
    let loaded = snapshot();
    let mut observed = snapshot();
    let mut outcome = SpuStepOutcome::Continue;
    let mut expected = expectation_of(&observed, &outcome);
    mutate(&mut expected, &mut observed, &mut outcome);
    let comparison = compare_reference(&expected, &loaded, &observed, &outcome)
        .unwrap_or_else(|error| panic!("{component:?}: {error}"));
    assert_eq!(
        comparison.compared,
        BTreeSet::from(ALL_COMPONENTS),
        "{component:?}"
    );
    assert!(comparison.unrepresented.is_empty(), "{component:?}");
    assert_eq!(comparison.differences, BTreeSet::from([component]));
}

fn hardware_capture(digest: &str) -> serde_json::Value {
    serde_json::json!({
        "kind": "hardware_capture",
        "capture_id": "capture-1",
        "device": "CECHA01",
        "environment": "firmware 4.93",
        "source_sha256": digest
    })
}

fn all_channels() -> serde_json::Value {
    serde_json::json!({
        "mfc_lsa": 1, "mfc_eah": 2, "mfc_eal": 3, "mfc_size": 4, "mfc_tag_id": 5,
        "tag_mask": 6, "tag_status": 7, "atomic_status": 8,
        "pending_mbox_rt": 127, "pending_get": [9, 10, 11, 12]
    })
}

#[test]
fn parse_refuses_unknown_keys_in_every_nested_object() {
    for pointer in [
        "/initial_state",
        "/provenance",
        "/expected/pc",
        "/expected",
        "",
    ] {
        let mut json = fixture();
        json.pointer_mut(pointer).expect(pointer)["unexpected"] = 1.into();
        assert!(
            matches!(parse(&json), Err(SpuReferenceError::Json(_))),
            "{pointer}"
        );
    }
    let mut json = fixture();
    let mut channels = all_channels();
    channels["mfc_cmd"] = 0.into();
    json["initial_state"]["channels"] = channels;
    assert!(matches!(parse(&json), Err(SpuReferenceError::Json(_))));
}

#[test]
fn parse_refuses_missing_provenance_fields_and_unknown_tag_names() {
    for key in ["citation", "vector_id"] {
        let mut json = fixture();
        json["provenance"]
            .as_object_mut()
            .expect("provenance object")
            .remove(key);
        assert!(
            matches!(parse(&json), Err(SpuReferenceError::Json(_))),
            "{key}"
        );
    }
    let mut json = fixture();
    json.as_object_mut()
        .expect("artifact object")
        .remove("provenance");
    assert!(matches!(parse(&json), Err(SpuReferenceError::Json(_))));
    let mut json = fixture();
    json["provenance"]["kind"] = "emulator_capture".into();
    assert!(matches!(parse(&json), Err(SpuReferenceError::Json(_))));
    let mut json = fixture();
    json["expected"]["outcome"]["value"] = "Continue".into();
    assert!(matches!(parse(&json), Err(SpuReferenceError::Json(_))));
}

#[test]
fn parse_refuses_malformed_json_with_the_serde_message() {
    let error = parse_reference_json("[").expect_err("malformed JSON must fail");
    let serde_message = serde_json::from_str::<SpuReferenceArtifact>("[")
        .expect_err("same input")
        .to_string();
    assert!(matches!(error, SpuReferenceError::Json(_)));
    assert_eq!(
        error.to_string(),
        format!("SPU reference JSON failed: {serde_message}")
    );
}

#[test]
fn parse_refuses_schema_version_zero_against_the_supported_constant() {
    let mut json = fixture();
    json["schema_version"] = 0.into();
    assert!(matches!(
        parse(&json),
        Err(SpuReferenceError::Version {
            found: 0,
            supported: SPU_REFERENCE_SCHEMA_VERSION
        })
    ));
    assert_eq!(artifact().schema_version, SPU_REFERENCE_SCHEMA_VERSION);
}

#[test]
fn parse_refuses_a_blank_case_id() {
    let mut json = fixture();
    json["case_id"] = " \t".into();
    assert!(matches!(
        parse(&json),
        Err(SpuReferenceError::Invalid { field: "case_id" })
    ));
}

#[test]
fn parse_refuses_word_lists_outside_the_local_store_contract() {
    let last_slot = (SPU_LS_SIZE - 4) as u32;
    for (name, words, pc) in [
        ("empty", 0_usize, 0_u32),
        ("sixty-five", 65, 0),
        ("unaligned", 1, 2),
        ("past the end", 2, last_slot),
        ("wrapping", 1, 0xffff_fffc),
    ] {
        let mut json = fixture();
        json["words"] = serde_json::json!(vec![IL_R3_1; words]);
        json["initial_state"]["pc"] = pc.into();
        assert!(
            matches!(
                parse(&json),
                Err(SpuReferenceError::Invalid { field: "words" })
            ),
            "{name}"
        );
    }
    for (name, words, pc) in [("last slot", 1_usize, last_slot), ("sixty-four", 64, 0)] {
        let mut json = fixture();
        json["words"] = serde_json::json!(vec![IL_R3_1; words]);
        json["initial_state"]["pc"] = pc.into();
        parse(&json).unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

#[test]
fn parse_refuses_non_canonical_register_keys_and_values_on_both_sides() {
    for (pointer, field) in [
        ("/initial_state/regs_hex", "initial_state.regs_hex"),
        ("/expected/regs_hex/value", "expected.regs_hex"),
    ] {
        for (key, hex) in [
            ("128", SEQUENTIAL.to_string()),
            ("-1", SEQUENTIAL.to_string()),
            (" 1", SEQUENTIAL.to_string()),
            ("1", SEQUENTIAL.to_uppercase()),
            ("1", SEQUENTIAL[1..].to_string()),
            ("1", format!("{SEQUENTIAL}0")),
            ("1", SEQUENTIAL.replace('0', "g")),
        ] {
            let mut json = fixture();
            json.pointer_mut(pointer).expect(pointer)[key] = hex.clone().into();
            assert!(
                matches!(
                    parse(&json),
                    Err(SpuReferenceError::Invalid { field: found }) if found == field
                ),
                "{field} {key} {hex}"
            );
        }
        let mut json = fixture();
        json.pointer_mut(pointer).expect(pointer)["127"] = SEQUENTIAL.into();
        parse(&json).unwrap_or_else(|error| panic!("{field}: {error}"));
    }
}

#[test]
fn parse_refuses_local_store_offsets_outside_the_store_on_both_sides() {
    for (pointer, field) in [
        ("/initial_state/local_store", "initial_state.local_store"),
        ("/expected/local_store/value", "expected.local_store"),
    ] {
        for key in ["262144", "007", "0x10", ""] {
            let mut json = fixture();
            json.pointer_mut(pointer).expect(pointer)[key] = 7.into();
            assert!(
                matches!(
                    parse(&json),
                    Err(SpuReferenceError::Invalid { field: found }) if found == field
                ),
                "{field} {key}"
            );
        }
        let mut json = fixture();
        json.pointer_mut(pointer).expect(pointer)["262143"] = 7.into();
        parse(&json).unwrap_or_else(|error| panic!("{field}: {error}"));
    }
}

#[test]
fn parse_refuses_unaligned_reservations_on_both_sides() {
    let mut json = fixture();
    json["initial_state"]["reservation"] = 0x81.into();
    assert!(matches!(
        parse(&json),
        Err(SpuReferenceError::Invalid {
            field: "initial_state.reservation"
        })
    ));
    let mut json = fixture();
    json["expected"]["reservation"] = serde_json::json!({"status": "value", "value": 0x81});
    assert!(matches!(
        parse(&json),
        Err(SpuReferenceError::Invalid {
            field: "expected.reservation"
        })
    ));
    let mut json = fixture();
    json["initial_state"]["reservation"] = 0x80.into();
    json["expected"]["reservation"] = serde_json::json!({"status": "value", "value": 0x80});
    parse(&json).expect("aligned reservations parse");
}

#[test]
fn parse_refuses_an_initial_mailbox_register_outside_the_bank() {
    let mut json = fixture();
    json["initial_state"]["channels"] = all_channels();
    json["initial_state"]["channels"]["pending_mbox_rt"] = 128.into();
    assert!(matches!(
        parse(&json),
        Err(SpuReferenceError::Invalid {
            field: "initial_state.channels.pending_mbox_rt"
        })
    ));
    json["initial_state"]["channels"]["pending_mbox_rt"] = 127.into();
    parse(&json).expect("register 127 is inside the bank");
}

#[test]
fn parse_refuses_a_blank_omission_reason_on_every_axis() {
    let fields = [
        ("/expected/regs_hex", "expected.regs_hex"),
        ("/expected/local_store", "expected.local_store"),
        ("/expected/pc", "expected.pc"),
        ("/expected/channels", "expected.channels"),
        ("/expected/reservation", "expected.reservation"),
        ("/expected/outcome", "expected.outcome"),
        ("/expected/effects", "expected.effects"),
        ("/expected/fault_discarded", "expected.fault_discarded"),
    ];
    assert_eq!(fields.len(), ALL_COMPONENTS.len());
    for (pointer, field) in fields {
        for (status, reason) in [("undefined", ""), ("unsupported", " \n")] {
            let mut json = fixture();
            *json.pointer_mut(pointer).expect(pointer) =
                serde_json::json!({"status": status, "reason": reason});
            assert!(
                matches!(
                    parse(&json),
                    Err(SpuReferenceError::Invalid { field: found }) if found == field
                ),
                "{pointer} {status}"
            );
        }
    }
}

#[test]
fn parse_refuses_blank_capture_text_and_non_canonical_digests() {
    for field in ["capture_id", "device", "environment"] {
        let mut json = fixture();
        json["provenance"] = hardware_capture(&"a".repeat(64));
        json["provenance"][field] = " ".into();
        assert!(
            matches!(
                parse(&json),
                Err(SpuReferenceError::Invalid {
                    field: "provenance"
                })
            ),
            "{field}"
        );
    }
    for digest in [
        "A".repeat(64),
        "a".repeat(63),
        "a".repeat(65),
        "g".repeat(64),
    ] {
        let mut json = fixture();
        json["provenance"] = hardware_capture(&digest);
        assert!(
            matches!(
                parse(&json),
                Err(SpuReferenceError::Invalid {
                    field: "provenance"
                })
            ),
            "{digest}"
        );
    }
    let mut json = fixture();
    json["provenance"] = hardware_capture(&"0123456789abcdef".repeat(4));
    let parsed = parse(&json).expect("lowercase 64-digit digest parses");
    assert!(matches!(
        parsed.provenance,
        SpuReferenceProvenance::HardwareCapture { .. }
    ));
}

#[test]
fn documented_vector_citations_are_exact_spu_isa_references() {
    for citation in [
        "SPU-ISA p:132 s:6. Shift and Rotate Instructions",
        "SPU-ISA p:1 s:x",
        "SPU-ISA p:65535 s:x",
    ] {
        assert!(valid_spu_citation(citation), "{citation}");
    }
    for citation in [
        "SPU-ISA p:0132 s:6",
        "SPU-ISA p:+132 s:6",
        "SPU-ISA p:65536 s:6",
        "SPU-ISA p:132 s: 6",
        "SPU-ISA p:132 s:6 ",
        "SPU-ISA p:132 s:",
        "SPU-ISA p:132",
        "SPU-ISA  p:132 s:6",
        "spu-isa p:132 s:6",
        "PPC-Book1 p:132 s:6",
        " SPU-ISA p:132 s:6",
        "",
    ] {
        assert!(!valid_spu_citation(citation), "{citation}");
    }
    let mut json = fixture();
    json["provenance"]["citation"] = "SPU-ISA p:0132 s:6".into();
    assert!(matches!(
        parse(&json),
        Err(SpuReferenceError::Invalid {
            field: "provenance"
        })
    ));
    let mut json = fixture();
    json["provenance"]["vector_id"] = " ".into();
    assert!(matches!(
        parse(&json),
        Err(SpuReferenceError::Invalid {
            field: "provenance"
        })
    ));
}

#[test]
fn error_display_text_is_pinned() {
    let decode_source = SpuDecodeError::Unsupported(u32::MAX);
    let cases = [
        (
            SpuReferenceError::Version {
                found: 0,
                supported: 1,
            },
            "SPU reference schema version 0 is unsupported; expected 1".to_string(),
        ),
        (
            SpuReferenceError::Invalid { field: "words" },
            "SPU reference field words is invalid".to_string(),
        ),
        (
            SpuReferenceError::Fetch { pc: 0x3_fffc },
            "SPU reference instruction fetch failed at PC 0x0003fffc".to_string(),
        ),
        (
            SpuReferenceError::Decode {
                pc: 4,
                source: SpuDecodeError::Unsupported(u32::MAX),
            },
            format!("SPU reference instruction at PC 0x00000004 did not decode: {decode_source}"),
        ),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text);
    }
    let decode = SpuReferenceError::Decode {
        pc: 4,
        source: SpuDecodeError::Unsupported(u32::MAX),
    };
    assert_eq!(
        std::error::Error::source(&decode).map(ToString::to_string),
        Some(decode_source.to_string())
    );
    assert!(std::error::Error::source(&SpuReferenceError::Fetch { pc: 0 }).is_none());
}

#[test]
fn artifact_types_round_trip_through_json() {
    let parsed = artifact();
    let json = serde_json::to_string(&parsed).expect("serializes");
    assert_eq!(parse_reference_json(&json).expect("round trip"), parsed);
    assert_eq!(
        serde_json::to_value(&parsed).expect("serializes"),
        fixture()
    );

    let mut json = fixture();
    json["provenance"] = hardware_capture(&"a".repeat(64));
    json["initial_state"]["channels"] = all_channels();
    json["initial_state"]["reservation"] = 0x80.into();
    json["expected"]["channels"] = serde_json::json!({"status": "value", "value": all_channels()});
    json["expected"]["reservation"] = serde_json::json!({"status": "value", "value": null});
    let capture = parse(&json).expect("capture parses");
    assert_eq!(serde_json::to_value(&capture).expect("serializes"), json);
    assert_eq!(
        capture.initial_state.channels,
        Some(SpuReferenceChannels {
            mfc_lsa: 1,
            mfc_eah: 2,
            mfc_eal: 3,
            mfc_size: 4,
            mfc_tag_id: 5,
            tag_mask: 6,
            tag_status: 7,
            atomic_status: 8,
            pending_mbox_rt: Some(127),
            pending_get: Some((9, 10, 11, 12)),
        })
    );
    assert_eq!(capture.expected.reservation, value(None));
}

#[test]
fn outcome_names_are_snake_case() {
    let cases = [
        (SpuReferenceOutcome::Continue, "continue"),
        (SpuReferenceOutcome::Branch, "branch"),
        (SpuReferenceOutcome::Yield, "yield"),
        (SpuReferenceOutcome::MemoryRead, "memory_read"),
        (SpuReferenceOutcome::Fault, "fault"),
    ];
    for (outcome, name) in cases {
        assert_eq!(
            serde_json::to_string(&outcome).expect("serializes"),
            format!("\"{name}\"")
        );
        assert_eq!(
            serde_json::from_str::<SpuReferenceOutcome>(&format!("\"{name}\"")).expect("parses"),
            outcome
        );
    }
}

#[test]
fn reference_field_states_serialize_with_a_status_tag() {
    let cases = [
        (
            value(BTreeMap::from([("4".to_string(), 7_u8)])),
            r#"{"status":"value","value":{"4":7}}"#,
        ),
        (
            ReferenceField::Undefined {
                reason: "unconstrained".into(),
            },
            r#"{"status":"undefined","reason":"unconstrained"}"#,
        ),
        (
            ReferenceField::Unsupported {
                reason: "unrepresented".into(),
            },
            r#"{"status":"unsupported","reason":"unrepresented"}"#,
        ),
    ];
    for (field, json) in cases {
        assert_eq!(serde_json::to_string(&field).expect("serializes"), json);
        let parsed: ReferenceField<BTreeMap<String, u8>> =
            serde_json::from_str(json).expect("parses");
        assert_eq!(parsed, field);
        assert_eq!(
            parsed.as_value(),
            match &field {
                ReferenceField::Value { value } => Some(value),
                ReferenceField::Undefined { .. } | ReferenceField::Unsupported { .. } => None,
            }
        );
    }
    assert!(serde_json::from_str::<ReferenceField<u32>>(r#"{"status":"absent"}"#).is_err());
    assert!(serde_json::from_str::<ReferenceField<u32>>(
        r#"{"status":"value","value":1,"reason":""}"#
    )
    .is_err());
}

#[test]
fn parse_index_accepts_only_canonical_in_range_decimals() {
    assert_eq!(parse_index("0", 128), Some(0));
    assert_eq!(parse_index("127", 128), Some(127));
    assert_eq!(parse_index("262143", SPU_LS_SIZE), Some(262_143));
    for key in [
        "128",
        "01",
        "+1",
        "-1",
        "",
        " 1",
        "1 ",
        "1.0",
        "0x1",
        "99999999999999999999",
    ] {
        assert_eq!(parse_index(key, 128), None, "{key}");
    }
}

#[test]
fn parse_register_decodes_thirty_two_lowercase_digits() {
    assert_eq!(
        parse_register(SEQUENTIAL),
        Some([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15])
    );
    assert_eq!(parse_register(&"f".repeat(32)), Some([0xff; 16]));
    for hex in [
        SEQUENTIAL.to_uppercase(),
        SEQUENTIAL[1..].to_string(),
        format!("{SEQUENTIAL}0"),
        SEQUENTIAL.replace('0', "g"),
        SEQUENTIAL.replace('0', " "),
        String::new(),
    ] {
        assert_eq!(parse_register(&hex), None, "{hex}");
    }
}

#[test]
fn a_complete_value_expectation_compares_every_component() {
    let loaded = snapshot();
    let outcome = SpuStepOutcome::Continue;
    let expected = expectation_of(&loaded, &outcome);
    let comparison =
        compare_reference(&expected, &loaded, &loaded, &outcome).expect("valid expectation");
    assert!(comparison.is_match(), "{:?}", comparison.differences);
    assert!(comparison.unrepresented.is_empty());
    assert_eq!(comparison.compared, BTreeSet::from(ALL_COMPONENTS));
}

#[test]
fn each_component_is_named_when_it_differs() {
    let cases: [(SpuReferenceComponent, Mutation); 8] = [
        (SpuReferenceComponent::Registers, |_, observed, _| {
            observed.regs[4][0] ^= 1;
        }),
        (SpuReferenceComponent::LocalStore, |_, observed, _| {
            observed.ls[0] ^= 1;
        }),
        (SpuReferenceComponent::ProgramCounter, |_, observed, _| {
            observed.pc += 4;
        }),
        (SpuReferenceComponent::Channels, |_, observed, _| {
            observed.channels.mfc_lsa ^= 1;
        }),
        (SpuReferenceComponent::Reservation, |_, observed, _| {
            observed.reservation = Some(ReservedLine::containing(0x80));
        }),
        (SpuReferenceComponent::Outcome, |_, _, outcome| {
            *outcome = SpuStepOutcome::Branch;
        }),
        (SpuReferenceComponent::Effects, |expected, _, outcome| {
            expected.outcome = value(SpuReferenceOutcome::Yield);
            *outcome = SpuStepOutcome::Yield {
                effects: vec![trace_marker()],
                reason: YieldReason::Syscall,
            };
        }),
        (
            SpuReferenceComponent::FaultDiscard,
            |expected, _, outcome| {
                expected.outcome = value(SpuReferenceOutcome::Fault);
                expected.fault_discarded = value(false);
                *outcome = SpuStepOutcome::Fault(SpuFault::LsOutOfRange(0));
            },
        ),
    ];
    for (component, mutate) in cases {
        single_difference(component, mutate);
    }
}

#[test]
fn every_channel_field_participates_in_the_channel_comparison() {
    let mutations: [fn(&mut SpuReferenceChannels); 10] = [
        |channels| channels.mfc_lsa ^= 1,
        |channels| channels.mfc_eah ^= 1,
        |channels| channels.mfc_eal ^= 1,
        |channels| channels.mfc_size ^= 1,
        |channels| channels.mfc_tag_id ^= 1,
        |channels| channels.tag_mask ^= 1,
        |channels| channels.tag_status ^= 1,
        |channels| channels.atomic_status ^= 1,
        |channels| channels.pending_mbox_rt = Some(3),
        |channels| channels.pending_get = Some((1, 2, 3, 4)),
    ];
    let loaded = snapshot();
    let outcome = SpuStepOutcome::Continue;
    for (index, mutate) in mutations.into_iter().enumerate() {
        let mut expected = expectation_of(&loaded, &outcome);
        let mut channels = SpuReferenceChannels::from(&loaded.channels);
        mutate(&mut channels);
        expected.channels = value(channels);
        let comparison =
            compare_reference(&expected, &loaded, &loaded, &outcome).expect("valid expectation");
        assert_eq!(
            comparison.differences,
            BTreeSet::from([SpuReferenceComponent::Channels]),
            "channel field {index}"
        );
    }
}

#[test]
fn channel_snapshots_convert_field_by_field() {
    let mut state = SpuState::new();
    state.channels.mfc_lsa = 1;
    state.channels.mfc_eah = 2;
    state.channels.mfc_eal = 3;
    state.channels.mfc_size = 4;
    state.channels.mfc_tag_id = 5;
    state.channels.tag_mask = 6;
    state.channels.tag_status = 7;
    state.channels.atomic_status = 8;
    state.channels.pending_mbox_rt = Some(9);
    state.channels.pending_get = Some((10, 11, 12, 13));
    let converted = SpuReferenceChannels::from(&SpuObservableSnapshot::capture(&state).channels);
    assert_eq!(
        converted,
        SpuReferenceChannels {
            mfc_lsa: 1,
            mfc_eah: 2,
            mfc_eal: 3,
            mfc_size: 4,
            mfc_tag_id: 5,
            tag_mask: 6,
            tag_status: 7,
            atomic_status: 8,
            pending_mbox_rt: Some(9),
            pending_get: Some((10, 11, 12, 13)),
        }
    );
}

#[test]
fn outcome_classes_map_from_step_outcomes() {
    let cases = [
        (SpuStepOutcome::Continue, SpuReferenceOutcome::Continue),
        (SpuStepOutcome::Branch, SpuReferenceOutcome::Branch),
        (
            SpuStepOutcome::Yield {
                effects: vec![trace_marker()],
                reason: YieldReason::DmaSubmitted,
            },
            SpuReferenceOutcome::Yield,
        ),
        (
            SpuStepOutcome::MemoryRead {
                ea: 0,
                lsa: 0,
                size: 16,
                acquire_line: None,
            },
            SpuReferenceOutcome::MemoryRead,
        ),
        (
            SpuStepOutcome::Fault(SpuFault::LsOutOfRange(0)),
            SpuReferenceOutcome::Fault,
        ),
    ];
    for (outcome, class) in &cases {
        assert_eq!(SpuReferenceOutcome::from(outcome), *class);
    }
    let loaded = snapshot();
    for (outcome, class) in &cases {
        let mut expected = expectation_of(&loaded, outcome);
        expected.effects = ReferenceField::Unsupported {
            reason: "payloads are not normalized".into(),
        };
        for (other, other_class) in &cases {
            expected.outcome = value(*other_class);
            let comparison = compare_reference(&expected, &loaded, &loaded, outcome)
                .expect("valid expectation for the outcome");
            let outcome_differs = comparison
                .differences
                .contains(&SpuReferenceComponent::Outcome);
            assert_eq!(
                outcome_differs,
                class != other_class,
                "{outcome:?} vs {other:?}"
            );
        }
    }
}

#[test]
fn effects_agree_only_when_a_yield_carries_none() {
    let loaded = snapshot();
    let empty_yield = SpuStepOutcome::Yield {
        effects: Vec::new(),
        reason: YieldReason::Finished,
    };
    let expected = expectation_of(&loaded, &empty_yield);
    let comparison =
        compare_reference(&expected, &loaded, &loaded, &empty_yield).expect("valid expectation");
    assert!(comparison.is_match(), "{:?}", comparison.differences);
    let full_yield = SpuStepOutcome::Yield {
        effects: vec![trace_marker()],
        reason: YieldReason::Finished,
    };
    let comparison =
        compare_reference(&expected, &loaded, &loaded, &full_yield).expect("valid expectation");
    assert_eq!(
        comparison.differences,
        BTreeSet::from([SpuReferenceComponent::Effects])
    );
}

#[test]
fn sparse_expected_overrides_apply_to_the_loaded_state() {
    let mut loaded = snapshot();
    loaded.regs[2] = parse_register(SEQUENTIAL).expect("sequential bytes");
    let mut observed = loaded.clone();
    observed.regs[4] = parse_register(ROTATED_12).expect("rotated bytes");
    observed.ls[3] = 9;
    let outcome = SpuStepOutcome::Continue;
    let mut expected = expectation_of(&observed, &outcome);
    expected.regs_hex = value(BTreeMap::from([("4".to_string(), ROTATED_12.to_string())]));
    expected.local_store = value(BTreeMap::from([("3".to_string(), 9_u8)]));
    let comparison =
        compare_reference(&expected, &loaded, &observed, &outcome).expect("valid expectation");
    assert!(comparison.is_match(), "{:?}", comparison.differences);

    let comparison =
        compare_reference(&expected, &observed, &observed, &outcome).expect("valid expectation");
    assert!(comparison.is_match(), "{:?}", comparison.differences);

    let mut without_overrides = expected.clone();
    without_overrides.regs_hex = value(BTreeMap::new());
    without_overrides.local_store = value(BTreeMap::new());
    let comparison = compare_reference(&without_overrides, &loaded, &observed, &outcome)
        .expect("valid expectation");
    assert_eq!(
        comparison.differences,
        BTreeSet::from([
            SpuReferenceComponent::Registers,
            SpuReferenceComponent::LocalStore
        ])
    );

    let mut nibble = expected.clone();
    nibble.regs_hex = value(BTreeMap::from([(
        "4".to_string(),
        ROTATED_12.replacen('b', "c", 1),
    )]));
    let comparison =
        compare_reference(&nibble, &loaded, &observed, &outcome).expect("valid expectation");
    assert_eq!(
        comparison.differences,
        BTreeSet::from([SpuReferenceComponent::Registers])
    );

    let mut byte = expected;
    byte.local_store = value(BTreeMap::from([("3".to_string(), 8_u8)]));
    let comparison =
        compare_reference(&byte, &loaded, &observed, &outcome).expect("valid expectation");
    assert_eq!(
        comparison.differences,
        BTreeSet::from([SpuReferenceComponent::LocalStore])
    );
}

#[test]
fn compare_refuses_malformed_expected_keys_before_comparing() {
    let loaded = snapshot();
    let outcome = SpuStepOutcome::Continue;
    let base = expectation_of(&loaded, &outcome);
    for (name, regs, field) in [
        ("register index", ("128", SEQUENTIAL), "expected.regs_hex"),
        ("register key", ("01", SEQUENTIAL), "expected.regs_hex"),
        ("register value", ("1", "short"), "expected.regs_hex"),
    ] {
        let mut expected = base.clone();
        expected.regs_hex = value(BTreeMap::from([(regs.0.to_string(), regs.1.to_string())]));
        let error = compare_reference(&expected, &loaded, &loaded, &outcome).expect_err(name);
        assert!(
            matches!(error, SpuReferenceError::Invalid { field: found } if found == field),
            "{name}: {error}"
        );
    }
    for key in ["262144", "01"] {
        let mut expected = base.clone();
        expected.local_store = value(BTreeMap::from([(key.to_string(), 0_u8)]));
        let error = compare_reference(&expected, &loaded, &loaded, &outcome).expect_err(key);
        assert!(
            matches!(
                error,
                SpuReferenceError::Invalid {
                    field: "expected.local_store"
                }
            ),
            "{key}: {error}"
        );
    }
    let mut expected = base;
    expected.effects = value(vec!["untyped".into()]);
    assert!(matches!(
        compare_reference(&expected, &loaded, &loaded, &outcome),
        Err(SpuReferenceError::Invalid {
            field: "expected.effects"
        })
    ));
}

#[test]
fn omitted_components_report_their_omission_kind_and_never_differ() {
    let loaded = snapshot();
    let mut observed = snapshot();
    observed.channels.mfc_lsa = 0x100;
    observed.reservation = Some(ReservedLine::containing(0x80));
    observed.pc = 8;
    let outcome = SpuStepOutcome::Continue;
    let mut expected = expectation_of(&loaded, &outcome);
    expected.channels = ReferenceField::Undefined {
        reason: "no channel rule".into(),
    };
    expected.reservation = ReferenceField::Unsupported {
        reason: "no reservation rule".into(),
    };
    expected.pc = ReferenceField::Undefined {
        reason: "no next-instruction rule".into(),
    };
    let comparison =
        compare_reference(&expected, &loaded, &observed, &outcome).expect("valid expectation");
    assert!(comparison.is_match(), "{:?}", comparison.differences);
    assert_eq!(
        comparison.unrepresented,
        BTreeMap::from([
            (
                SpuReferenceComponent::Channels,
                SpuReferenceOmission::Undefined
            ),
            (
                SpuReferenceComponent::Reservation,
                SpuReferenceOmission::Unsupported
            ),
            (
                SpuReferenceComponent::ProgramCounter,
                SpuReferenceOmission::Undefined
            ),
        ])
    );
    assert_eq!(comparison.compared.len(), ALL_COMPONENTS.len() - 3);
    for component in comparison.unrepresented.keys() {
        assert!(!comparison.compared.contains(component), "{component:?}");
    }
}

#[test]
fn is_match_ignores_unrepresented_components_but_not_differences() {
    let comparison = SpuReferenceComparison {
        compared: BTreeSet::new(),
        differences: BTreeSet::new(),
        unrepresented: BTreeMap::from([(
            SpuReferenceComponent::Channels,
            SpuReferenceOmission::Unsupported,
        )]),
    };
    assert!(comparison.is_match());
    let comparison = SpuReferenceComparison {
        compared: BTreeSet::from([SpuReferenceComponent::ProgramCounter]),
        differences: BTreeSet::from([SpuReferenceComponent::ProgramCounter]),
        unrepresented: BTreeMap::new(),
    };
    assert!(!comparison.is_match());
}

#[test]
fn replay_of_the_committed_fixture_loads_words_and_compares_six_components() {
    let artifact = artifact();
    let replay = replay_reference(&artifact).expect("fixture replays");
    assert!(replay.comparison.is_match(), "{:?}", replay.comparison);
    assert_eq!(replay.comparison.compared.len(), 6);
    assert_eq!(replay.comparison.unrepresented.len(), 2);
    assert_eq!(replay.outcome, SpuStepOutcome::Continue);
    assert_eq!(
        replay.initial.ls[..4],
        artifact.words[0].to_be_bytes(),
        "words load at the initial PC"
    );
    assert_eq!(
        replay.initial.regs[2],
        parse_register(SEQUENTIAL).expect("sequential bytes")
    );
    assert_eq!(replay.initial.pc, 0);
    assert_eq!(replay.state.pc, 4);
    assert_eq!(
        replay.state.regs[4],
        parse_register(ROTATED_12).expect("rotated bytes")
    );
    assert_eq!(replay.state.ls, replay.initial.ls);
}

#[test]
fn replay_names_the_mismatch_of_an_altered_fixture() {
    let mut altered = artifact();
    altered.expected.pc = value(8);
    let replay = replay_reference(&altered).expect("altered expectation still replays");
    assert_eq!(
        replay.comparison.differences,
        BTreeSet::from([SpuReferenceComponent::ProgramCounter])
    );

    let mut altered = artifact();
    altered.expected.regs_hex = value(BTreeMap::from([("4".to_string(), SEQUENTIAL.to_string())]));
    let replay = replay_reference(&altered).expect("altered expectation still replays");
    assert_eq!(
        replay.comparison.differences,
        BTreeSet::from([SpuReferenceComponent::Registers])
    );

    let mut altered = artifact();
    altered.expected.outcome = value(SpuReferenceOutcome::Branch);
    let replay = replay_reference(&altered).expect("altered expectation still replays");
    assert_eq!(
        replay.comparison.differences,
        BTreeSet::from([SpuReferenceComponent::Outcome])
    );
}

#[test]
fn replay_validates_the_artifact_before_executing() {
    let mut altered = artifact();
    altered.schema_version = 2;
    assert!(matches!(
        replay_reference(&altered),
        Err(SpuReferenceError::Version {
            found: 2,
            supported: 1
        })
    ));
    let mut altered = artifact();
    altered.words.clear();
    assert!(matches!(
        replay_reference(&altered),
        Err(SpuReferenceError::Invalid { field: "words" })
    ));
}

#[test]
fn replay_places_words_at_a_nonzero_pc_and_applies_initial_overrides() {
    let mut altered = artifact();
    altered.initial_state.pc = 0x100;
    altered
        .initial_state
        .local_store
        .insert("7".to_string(), 0xaa);
    altered.initial_state.reservation = Some(0x80);
    altered.initial_state.channels = Some(SpuReferenceChannels {
        mfc_lsa: 1,
        mfc_eah: 2,
        mfc_eal: 3,
        mfc_size: 4,
        mfc_tag_id: 5,
        tag_mask: 6,
        tag_status: 7,
        atomic_status: 8,
        pending_mbox_rt: Some(9),
        pending_get: Some((10, 11, 12, 13)),
    });
    altered.expected.pc = value(0x104);
    altered.expected.reservation = value(Some(0x80));
    altered.expected.channels = value(altered.initial_state.channels.clone().expect("set"));
    let replay = replay_reference(&altered).expect("relocated vector replays");
    assert!(replay.comparison.is_match(), "{:?}", replay.comparison);
    assert_eq!(replay.comparison.compared.len(), ALL_COMPONENTS.len());
    assert_eq!(
        replay.initial.ls[0x100..0x104],
        altered.words[0].to_be_bytes()
    );
    assert_eq!(replay.initial.ls[..4], [0; 4]);
    assert_eq!(replay.initial.ls[7], 0xaa);
    assert_eq!(replay.state.ls[7], 0xaa);
    assert_eq!(
        replay.state.reservation,
        Some(ReservedLine::containing(0x80))
    );
    assert_eq!(replay.state.channels.mfc_lsa, 1);
    assert_eq!(replay.state.channels.pending_get, Some((10, 11, 12, 13)));
}

#[test]
fn replay_stops_at_a_yield_before_later_words_execute() {
    let mut altered = artifact();
    altered.words = vec![STOP_WORD, IL_R3_1];
    altered.expected.regs_hex = value(BTreeMap::new());
    altered.expected.pc = value(0);
    altered.expected.outcome = value(SpuReferenceOutcome::Yield);
    let replay = replay_reference(&altered).expect("stop replays");
    assert!(
        matches!(replay.outcome, SpuStepOutcome::Yield { .. }),
        "{:?}",
        replay.outcome
    );
    assert_eq!(replay.state.pc, 0);
    assert_eq!(replay.state.regs[3], [0; 16]);
    assert!(replay.comparison.is_match(), "{:?}", replay.comparison);

    let mut altered = artifact();
    altered.words = vec![IL_R3_1, STOP_WORD];
    // [SPU-ISA p:52 s:4. Constant-Formation Instructions] il replicates the sign-extended I16 into all four word slots.
    altered.expected.regs_hex = value(BTreeMap::from([(
        "3".to_string(),
        "00000001000000010000000100000001".to_string(),
    )]));
    altered.expected.pc = value(4);
    altered.expected.outcome = value(SpuReferenceOutcome::Yield);
    let replay = replay_reference(&altered).expect("il then stop replays");
    assert!(replay.comparison.is_match(), "{:?}", replay.comparison);
}

#[test]
fn replay_restores_the_loaded_state_after_a_fault() {
    let mut altered = artifact();
    altered.words = vec![IL_R3_1, RCHCNT_R3_CH127];
    altered.expected.regs_hex = value(BTreeMap::new());
    altered.expected.pc = value(0);
    altered.expected.outcome = value(SpuReferenceOutcome::Fault);
    altered.expected.fault_discarded = value(true);
    let replay = replay_reference(&altered).expect("faulting vector replays");
    assert!(
        matches!(replay.outcome, SpuStepOutcome::Fault(_)),
        "{:?}",
        replay.outcome
    );
    assert_eq!(replay.state, replay.initial);
    assert!(replay.comparison.is_match(), "{:?}", replay.comparison);

    altered.expected.fault_discarded = value(false);
    let replay = replay_reference(&altered).expect("faulting vector replays");
    assert_eq!(
        replay.comparison.differences,
        BTreeSet::from([SpuReferenceComponent::FaultDiscard])
    );
}

#[test]
fn replay_names_the_program_counter_of_a_later_undecodable_word() {
    let mut altered = artifact();
    altered.words = vec![IL_R3_1, u32::MAX];
    let error = replay_reference(&altered).expect_err("second word cannot decode");
    assert!(matches!(
        error,
        SpuReferenceError::Decode {
            pc: 4,
            source: SpuDecodeError::Unsupported(u32::MAX)
        }
    ));
}
