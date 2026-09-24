use std::collections::BTreeSet;

use super::compare::{reference_yield, render_effects};
use super::*;

use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::{FaultRegisterDump, YieldReason};
use cellgov_sync::ReservedLine;

use crate::ppu_paths::PpuPathRun;

const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/ppu_reference/li_r3_7_v1.json"
));

const ALL_COMPONENTS: [PpuReferenceComponent; 27] = [
    PpuReferenceComponent::StateGpr,
    PpuReferenceComponent::StateFpr,
    PpuReferenceComponent::StateVr,
    PpuReferenceComponent::StatePc,
    PpuReferenceComponent::StateCr,
    PpuReferenceComponent::StateLr,
    PpuReferenceComponent::StateCtr,
    PpuReferenceComponent::StateXer,
    PpuReferenceComponent::StateVrsave,
    PpuReferenceComponent::StateTb,
    PpuReferenceComponent::StateReservation,
    PpuReferenceComponent::Memory,
    PpuReferenceComponent::StopReason,
    PpuReferenceComponent::StopFault,
    PpuReferenceComponent::StopPc,
    PpuReferenceComponent::StopLr,
    PpuReferenceComponent::StopSyscallLev,
    PpuReferenceComponent::StopFaultingEa,
    PpuReferenceComponent::StopFaultRegisters,
    PpuReferenceComponent::StopSyscallArgs,
    PpuReferenceComponent::Retired,
    PpuReferenceComponent::StagedEffects,
    PpuReferenceComponent::CommittedEffects,
    PpuReferenceComponent::Reservations,
    PpuReferenceComponent::StoreBuffer,
    PpuReferenceComponent::CommitError,
    PpuReferenceComponent::FaultDiscarded,
];

type Mutation = fn(&mut PpuReferenceObservation, &mut PpuPathRun);

fn fixture() -> serde_json::Value {
    serde_json::from_str(FIXTURE).expect("fixture is JSON")
}

fn parse(json: &serde_json::Value) -> Result<PpuReferenceArtifact, PpuReferenceError> {
    parse_reference_json(&json.to_string())
}

fn artifact() -> PpuReferenceArtifact {
    parse_reference_json(FIXTURE).expect("fixture parses")
}

fn fixture_run() -> PpuPathRun {
    replay_reference(&artifact())
        .expect("fixture replays")
        .runs
        .remove(0)
}

fn value<T>(value: T) -> ReferenceField<T> {
    ReferenceField::Value { value }
}

fn hex_vector(low: u128) -> String {
    format!("{low:032x}")
}

fn observation_of(run: &PpuPathRun) -> PpuReferenceObservation {
    let state = &run.observation.state;
    let fault = match run.stop.fault.as_ref() {
        None => PpuReferenceFault::None,
        Some(FaultKind::Validation) => PpuReferenceFault::Validation,
        Some(FaultKind::Guest(code)) => PpuReferenceFault::Guest { code: *code },
    };
    let fault_registers =
        run.stop
            .diagnostics
            .fault_regs
            .as_ref()
            .map(|registers| PpuReferenceFaultRegisters {
                gpr: registers.gprs.to_vec(),
                lr: registers.lr,
                ctr: registers.ctr,
                xer: registers.xer,
                cr: registers.cr,
            });
    PpuReferenceObservation {
        state: PpuReferenceState {
            gpr: value(state.gpr.to_vec()),
            fpr: value(state.fpr.to_vec()),
            vr_hex: value(state.vr.iter().map(|vr| hex_vector(*vr)).collect()),
            pc: value(state.pc),
            cr: value(state.cr),
            lr: value(state.lr),
            ctr: value(state.ctr),
            xer: value(state.xer),
            vrsave: value(state.vrsave),
            tb: value(state.tb),
            reservation: value(state.reservation.map(|line| line.addr())),
        },
        memory: value(run.observation.memory.clone()),
        stop: PpuReferenceStop {
            reason: value(reference_yield(run.stop.reason)),
            fault: value(fault),
            pc: value(run.stop.pc),
            lr: value(run.stop.diagnostics.lr),
            syscall_lev: value(run.stop.diagnostics.syscall_lev),
            faulting_ea: value(run.stop.diagnostics.faulting_ea),
            fault_registers: value(fault_registers),
            syscall_args: value(run.stop.syscall_args.map(|args| args.to_vec())),
        },
        retired: value(run.retired),
        staged_effects: value(render_effects(&run.observation.staged_effects)),
        committed_effects: value(render_effects(&run.observation.committed_effects)),
        reservations: value(
            run.observation
                .reservations
                .iter()
                .map(|(unit, line)| (unit.raw(), line.addr()))
                .collect(),
        ),
        store_buffer: value(
            run.observation
                .store_buffer
                .iter()
                .map(|store| format!("{store:?}"))
                .collect(),
        ),
        commit_error: value(
            run.observation
                .commit_error
                .as_ref()
                .map(ToString::to_string),
        ),
        fault_discarded: value(run.observation.fault_discarded),
    }
}

fn single_named_difference(
    component: PpuReferenceComponent,
    mutate: Mutation,
) -> PpuReferenceDifference {
    let mut run = fixture_run();
    let mut expected = observation_of(&run);
    mutate(&mut expected, &mut run);
    let comparison = compare_reference(&expected, &run);
    assert_eq!(
        comparison.differences.len(),
        1,
        "{component:?}: {:?}",
        comparison.differences
    );
    assert!(comparison.unrepresented.is_empty(), "{component:?}");
    assert_eq!(
        comparison.compared.len(),
        ALL_COMPONENTS.len(),
        "{component:?}"
    );
    let difference = comparison.differences[0].clone();
    assert_eq!(difference.field, component);
    difference
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

#[test]
fn parse_refuses_unknown_keys_inside_reference_fields_and_provenance() {
    let mut json = fixture();
    json["expected"]["state"]["pc"]["extra"] = 1.into();
    assert!(matches!(parse(&json), Err(PpuReferenceError::Json(_))));

    let mut json = fixture();
    json["provenance"]["device"] = "CECHA01".into();
    assert!(matches!(parse(&json), Err(PpuReferenceError::Json(_))));

    let mut json = fixture();
    json["initial_state"]["msr"] = 0.into();
    assert!(matches!(parse(&json), Err(PpuReferenceError::Json(_))));
}

#[test]
fn parse_refuses_unknown_tags_and_non_snake_case_names() {
    let mut json = fixture();
    json["provenance"]["kind"] = "emulator_capture".into();
    assert!(matches!(parse(&json), Err(PpuReferenceError::Json(_))));

    let mut json = fixture();
    json["expected"]["state"]["pc"]["status"] = "missing".into();
    assert!(matches!(parse(&json), Err(PpuReferenceError::Json(_))));

    let mut json = fixture();
    json["expected"]["stop"]["reason"]["value"] = "BudgetExhausted".into();
    assert!(matches!(parse(&json), Err(PpuReferenceError::Json(_))));

    let mut json = fixture();
    json["expected"]["stop"]["fault"]["value"]["kind"] = "None".into();
    assert!(matches!(parse(&json), Err(PpuReferenceError::Json(_))));

    let mut json = fixture();
    json["initial_state"]["base"] = "Zeroed".into();
    assert!(matches!(parse(&json), Err(PpuReferenceError::Json(_))));
}

#[test]
fn parse_refuses_a_missing_citation_or_vector_id() {
    let mut json = fixture();
    json["provenance"]
        .as_object_mut()
        .expect("provenance object")
        .remove("citation");
    assert!(matches!(parse(&json), Err(PpuReferenceError::Json(_))));

    let mut json = fixture();
    json["provenance"]
        .as_object_mut()
        .expect("provenance object")
        .remove("vector_id");
    assert!(matches!(parse(&json), Err(PpuReferenceError::Json(_))));

    let mut json = fixture();
    json.as_object_mut()
        .expect("artifact object")
        .remove("provenance");
    assert!(matches!(parse(&json), Err(PpuReferenceError::Json(_))));
}

#[test]
fn parse_refuses_malformed_json_with_the_serde_message() {
    let error = parse_reference_json("{").expect_err("malformed JSON must fail");
    let serde_message = serde_json::from_str::<PpuReferenceArtifact>("{")
        .expect_err("same input")
        .to_string();
    assert!(matches!(error, PpuReferenceError::Json(_)));
    assert_eq!(
        error.to_string(),
        format!("PPU reference JSON failed: {serde_message}")
    );
}

#[test]
fn parse_refuses_schema_version_zero_against_the_supported_constant() {
    let mut json = fixture();
    json["schema_version"] = 0.into();
    assert!(matches!(
        parse(&json),
        Err(PpuReferenceError::Version {
            found: 0,
            supported: PPU_REFERENCE_SCHEMA_VERSION
        })
    ));
    assert_eq!(artifact().schema_version, PPU_REFERENCE_SCHEMA_VERSION);
}

#[test]
fn parse_refuses_a_data_base_outside_the_path_contract() {
    let mut json = fixture();
    json["memory_base"] = 0.into();
    assert!(matches!(
        parse(&json),
        Err(PpuReferenceError::MemoryBase {
            found: 0,
            expected: 0x1000_0000
        })
    ));
    json["memory_base"] = 0x1000_0080_u64.into();
    assert!(matches!(
        parse(&json),
        Err(PpuReferenceError::MemoryBase {
            found: 0x1000_0080,
            ..
        })
    ));
}

#[test]
fn parse_refuses_sparse_register_indices_outside_each_bank() {
    for (bank, name, entry) in [
        ("gpr", "GPR", serde_json::json!(1)),
        ("fpr", "FPR", serde_json::json!(1)),
        ("vr_hex", "VR", serde_json::json!(hex_vector(1))),
    ] {
        let mut json = fixture();
        json["initial_state"][bank]["32"] = entry.clone();
        assert!(
            matches!(
                parse(&json),
                Err(PpuReferenceError::RegisterIndex { bank: found, index: 32 }) if found == name
            ),
            "{bank}"
        );
        let mut json = fixture();
        json["initial_state"][bank]["31"] = entry;
        parse(&json).unwrap_or_else(|error| panic!("{bank} index 31 must parse: {error}"));
    }
}

#[test]
fn parse_refuses_non_canonical_vector_register_values() {
    let mut json = fixture();
    json["initial_state"]["vr_hex"]["1"] = hex_vector(0xabc).to_uppercase().into();
    assert!(matches!(
        parse(&json),
        Err(PpuReferenceError::VectorValue { index: 1 })
    ));

    let mut json = fixture();
    json["initial_state"]["vr_hex"]["1"] = format!("{}0", hex_vector(1)).into();
    assert!(matches!(
        parse(&json),
        Err(PpuReferenceError::VectorValue { index: 1 })
    ));

    let mut bank = vec![hex_vector(0); 32];
    bank[5] = hex_vector(0)[1..].to_string();
    let mut json = fixture();
    json["expected"]["state"]["vr_hex"] = serde_json::json!({"status": "value", "value": bank});
    assert!(matches!(
        parse(&json),
        Err(PpuReferenceError::VectorValue { index: 5 })
    ));

    let mut json = fixture();
    json["initial_state"]["vr_hex"]["1"] = hex_vector(0xabc).into();
    json["expected"]["state"]["vr_hex"] =
        serde_json::json!({"status": "value", "value": vec![hex_vector(0); 32]});
    parse(&json).expect("lowercase fixed-width hex parses");
}

#[test]
fn parse_refuses_an_unaligned_initial_reservation() {
    let mut json = fixture();
    json["initial_state"]["reservation"] = 0x81.into();
    assert!(matches!(
        parse(&json),
        Err(PpuReferenceError::ReservationAlignment { address: 0x81 })
    ));
    json["initial_state"]["reservation"] = 0x80.into();
    parse(&json).expect("aligned reservation parses");
}

#[test]
fn parse_refuses_register_banks_and_stop_lists_with_the_wrong_length() {
    for (bank, found) in [("gpr", 31), ("fpr", 33), ("vr_hex", 1)] {
        let mut json = fixture();
        let entry = if bank == "vr_hex" {
            serde_json::json!(hex_vector(0))
        } else {
            serde_json::json!(0)
        };
        json["expected"]["state"][bank] =
            serde_json::json!({"status": "value", "value": vec![entry; found]});
        let error = parse(&json).expect_err(bank);
        match error {
            PpuReferenceError::FieldLength {
                field,
                found: seen,
                expected,
            } => {
                assert_eq!(field, format!("state.{bank}"));
                assert_eq!((seen, expected), (found, 32), "{bank}");
            }
            other => panic!("{bank}: {other}"),
        }
    }

    let mut json = fixture();
    json["expected"]["stop"]["fault_registers"] = serde_json::json!({
        "status": "value",
        "value": {"gpr": [0, 0], "lr": 0, "ctr": 0, "xer": 0, "cr": 0}
    });
    assert!(matches!(
        parse(&json),
        Err(PpuReferenceError::FieldLength {
            field: "stop.fault_registers.gpr",
            found: 2,
            expected: 32
        })
    ));

    let mut json = fixture();
    json["expected"]["stop"]["syscall_args"] =
        serde_json::json!({"status": "value", "value": vec![0; 8]});
    assert!(matches!(
        parse(&json),
        Err(PpuReferenceError::FieldLength {
            field: "stop.syscall_args",
            found: 8,
            expected: 9
        })
    ));
}

#[test]
fn parse_refuses_values_schema_version_one_cannot_normalize() {
    for field in ["staged_effects", "store_buffer"] {
        let mut json = fixture();
        json["expected"][field]["value"] = serde_json::json!(["text"]);
        assert!(
            matches!(
                parse(&json),
                Err(PpuReferenceError::UnsupportedValue { field: found }) if found == field
            ),
            "{field}"
        );
    }
    let mut json = fixture();
    json["expected"]["commit_error"]["value"] = "refused".into();
    assert!(matches!(
        parse(&json),
        Err(PpuReferenceError::UnsupportedValue {
            field: "commit_error"
        })
    ));
}

#[test]
fn parse_refuses_a_blank_omission_reason_on_every_field() {
    let fields = [
        ("/expected/state/gpr", "state.gpr"),
        ("/expected/state/fpr", "state.fpr"),
        ("/expected/state/vr_hex", "state.vr_hex"),
        ("/expected/state/pc", "state.pc"),
        ("/expected/state/cr", "state.cr"),
        ("/expected/state/lr", "state.lr"),
        ("/expected/state/ctr", "state.ctr"),
        ("/expected/state/xer", "state.xer"),
        ("/expected/state/vrsave", "state.vrsave"),
        ("/expected/state/tb", "state.tb"),
        ("/expected/state/reservation", "state.reservation"),
        ("/expected/memory", "memory"),
        ("/expected/stop/reason", "stop.reason"),
        ("/expected/stop/fault", "stop.fault"),
        ("/expected/stop/pc", "stop.pc"),
        ("/expected/stop/lr", "stop.lr"),
        ("/expected/stop/syscall_lev", "stop.syscall_lev"),
        ("/expected/stop/faulting_ea", "stop.faulting_ea"),
        ("/expected/stop/fault_registers", "stop.fault_registers"),
        ("/expected/stop/syscall_args", "stop.syscall_args"),
        ("/expected/retired", "retired"),
        ("/expected/staged_effects", "staged_effects"),
        ("/expected/committed_effects", "committed_effects"),
        ("/expected/reservations", "reservations"),
        ("/expected/store_buffer", "store_buffer"),
        ("/expected/commit_error", "commit_error"),
        ("/expected/fault_discarded", "fault_discarded"),
    ];
    assert_eq!(fields.len(), ALL_COMPONENTS.len());
    for (pointer, field) in fields {
        for (status, reason) in [("undefined", ""), ("unsupported", "\t\n")] {
            let mut json = fixture();
            *json.pointer_mut(pointer).expect(pointer) =
                serde_json::json!({"status": status, "reason": reason});
            assert!(
                matches!(
                    parse(&json),
                    Err(PpuReferenceError::EmptyReason { field: found, status: seen })
                        if found == field && seen == status
                ),
                "{pointer} {status}"
            );
        }
    }
}

#[test]
fn parse_refuses_blank_provenance_text() {
    let mut json = fixture();
    json["provenance"]["vector_id"] = "  ".into();
    assert!(matches!(
        parse(&json),
        Err(PpuReferenceError::EmptyProvenance { field: "vector_id" })
    ));
    for field in ["capture_id", "device", "environment"] {
        let mut json = fixture();
        json["provenance"] = hardware_capture(&"a".repeat(64));
        json["provenance"][field] = "".into();
        assert!(
            matches!(
                parse(&json),
                Err(PpuReferenceError::EmptyProvenance { field: found }) if found == field
            ),
            "{field}"
        );
    }
}

#[test]
fn parse_refuses_non_canonical_capture_digests() {
    for digest in [
        "A".repeat(64),
        "a".repeat(63),
        "a".repeat(65),
        "g".repeat(64),
    ] {
        let mut json = fixture();
        json["provenance"] = hardware_capture(&digest);
        assert!(
            matches!(parse(&json), Err(PpuReferenceError::CaptureDigest)),
            "{digest}"
        );
    }
    let mut json = fixture();
    json["provenance"] = hardware_capture(&"0123456789abcdef".repeat(4));
    let parsed = parse(&json).expect("lowercase 64-digit digest parses");
    assert!(matches!(
        parsed.provenance,
        PpuReferenceProvenance::HardwareCapture { .. }
    ));
}

#[test]
fn documented_vector_citations_need_a_supported_key_page_and_section() {
    for citation in [
        "PPC-Book1 p:51",
        "PPC-Book1 s:3.3.8",
        "ppc-book1 p:51 s:3.3.8",
        "CBEA p:51 s:3.3.8",
        "SPU-ISA p:51 s:3.3.8",
        "",
    ] {
        let mut json = fixture();
        json["provenance"]["citation"] = citation.into();
        assert!(
            matches!(
                parse(&json),
                Err(PpuReferenceError::Citation { citation: found }) if found == citation
            ),
            "{citation}"
        );
    }
    for citation in [
        "PPC-Book1 p:51 s:3.3.8",
        "PPC-Book2 p:1 s:2",
        "PPC-Book3 p:1 s:2",
        "AltiVec-PEM p:1 s:2",
    ] {
        let mut json = fixture();
        json["provenance"]["citation"] = citation.into();
        parse(&json).unwrap_or_else(|error| panic!("{citation}: {error}"));
    }
}

#[test]
fn error_display_text_is_pinned() {
    let cases = [
        (
            PpuReferenceError::Version {
                found: 0,
                supported: 1,
            },
            "PPU reference schema version 0 is unsupported; expected 1",
        ),
        (
            PpuReferenceError::FieldLength {
                field: "state.gpr",
                found: 31,
                expected: 32,
            },
            "PPU reference field state.gpr has 31 entries; expected 32",
        ),
        (
            PpuReferenceError::RegisterIndex {
                bank: "GPR",
                index: 32,
            },
            "PPU reference GPR index 32 is outside 0..32",
        ),
        (
            PpuReferenceError::VectorValue { index: 5 },
            "PPU reference vector register 5 must contain 32 lowercase hexadecimal digits",
        ),
        (
            PpuReferenceError::ReservationAlignment { address: 0x81 },
            "PPU reference reservation 0x0000000000000081 is not 128-byte aligned",
        ),
        (
            PpuReferenceError::MemoryBase {
                found: 0,
                expected: 0x1000_0000,
            },
            "PPU reference data base 0x0000000000000000 is unsupported; \
             expected 0x0000000010000000",
        ),
        (
            PpuReferenceError::CaptureDigest,
            "PPU hardware capture SHA-256 must contain 64 lowercase hexadecimal digits",
        ),
        (
            PpuReferenceError::Citation {
                citation: "informal note".into(),
            },
            "PPU documented vector citation is not a supported official-source citation: \
             informal note",
        ),
        (
            PpuReferenceError::EmptyReason {
                field: "state.tb",
                status: "undefined",
            },
            "PPU reference field state.tb has an empty undefined reason",
        ),
        (
            PpuReferenceError::EmptyProvenance { field: "vector_id" },
            "PPU reference provenance field vector_id must not be empty",
        ),
        (
            PpuReferenceError::UnsupportedValue {
                field: "commit_error",
            },
            "PPU reference schema version one cannot represent non-empty field \
             commit_error; mark it unsupported",
        ),
        (
            PpuReferenceError::Replay(crate::ppu_paths::PpuPathError::EmptySequence),
            "PPU reference replay failed: PPU path sequence must contain at least one \
             instruction",
        ),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text);
    }
}

#[test]
fn reference_field_states_serialize_with_a_status_tag() {
    let cases = [
        (value(7_u64), r#"{"status":"value","value":7}"#),
        (
            ReferenceField::Undefined {
                reason: "not constrained".into(),
            },
            r#"{"status":"undefined","reason":"not constrained"}"#,
        ),
        (
            ReferenceField::Unsupported {
                reason: "not represented".into(),
            },
            r#"{"status":"unsupported","reason":"not represented"}"#,
        ),
    ];
    for (field, json) in cases {
        assert_eq!(serde_json::to_string(&field).expect("serializes"), json);
        let parsed: ReferenceField<u64> = serde_json::from_str(json).expect("parses");
        assert_eq!(parsed, field);
    }
    assert_eq!(
        serde_json::to_string(&value(None::<u64>)).expect("serializes"),
        r#"{"status":"value","value":null}"#
    );
    for json in [
        r#"{"status":"value","value":1,"reason":"x"}"#,
        r#"{"status":"undefined","reason":"x","value":1}"#,
        r#"{"status":"present","value":1}"#,
        r#"{"value":1}"#,
        r#"{"status":"undefined"}"#,
    ] {
        assert!(
            serde_json::from_str::<ReferenceField<u64>>(json).is_err(),
            "{json}"
        );
    }
}

#[test]
fn artifact_types_round_trip_through_json() {
    let parsed = artifact();
    let json = serde_json::to_string(&parsed).expect("serializes");
    assert_eq!(parse_reference_json(&json).expect("round trip"), parsed);

    let mut capture = fixture();
    capture["provenance"] = hardware_capture(&"a".repeat(64));
    let capture = parse(&capture).expect("capture parses");
    assert_eq!(
        serde_json::to_value(&capture.provenance).expect("serializes"),
        hardware_capture(&"a".repeat(64))
    );
    assert_eq!(
        serde_json::to_string(&parsed.provenance).expect("serializes"),
        r#"{"kind":"documented_vector","citation":"PPC-Book1 p:51 s:3.3.8","vector_id":"li-r3-7"}"#
    );

    let mut stop = parsed.expected.stop.clone();
    stop.fault = value(PpuReferenceFault::Guest { code: 5 });
    stop.fault_registers = value(Some(PpuReferenceFaultRegisters {
        gpr: vec![0; 32],
        lr: 1,
        ctr: 2,
        xer: 3,
        cr: 4,
    }));
    stop.syscall_args = value(Some(vec![9; 9]));
    let json = serde_json::to_value(&stop).expect("serializes");
    assert_eq!(
        json["fault"],
        serde_json::json!({"status": "value", "value": {"kind": "guest", "code": 5}})
    );
    assert_eq!(
        serde_json::from_value::<PpuReferenceStop>(json).expect("round trip"),
        stop
    );
    assert_eq!(
        serde_json::to_string(&PpuReferenceFault::Validation).expect("serializes"),
        r#"{"kind":"validation"}"#
    );
    assert_eq!(
        serde_json::to_string(&PpuReferenceStateBase::Zeroed).expect("serializes"),
        r#""zeroed""#
    );
}

#[test]
fn yield_reason_names_are_snake_case() {
    let cases = [
        (PpuReferenceYieldReason::BudgetExhausted, "budget_exhausted"),
        (PpuReferenceYieldReason::Syscall, "syscall"),
        (PpuReferenceYieldReason::MailboxAccess, "mailbox_access"),
        (PpuReferenceYieldReason::DmaSubmitted, "dma_submitted"),
        (PpuReferenceYieldReason::DmaWait, "dma_wait"),
        (PpuReferenceYieldReason::WaitingSync, "waiting_sync"),
        (PpuReferenceYieldReason::Fault, "fault"),
        (
            PpuReferenceYieldReason::InterruptBoundary,
            "interrupt_boundary",
        ),
        (PpuReferenceYieldReason::Finished, "finished"),
    ];
    for (reason, name) in cases {
        assert_eq!(
            serde_json::to_string(&reason).expect("serializes"),
            format!("\"{name}\"")
        );
        assert_eq!(
            serde_json::from_str::<PpuReferenceYieldReason>(&format!("\"{name}\""))
                .expect("parses"),
            reason
        );
    }
}

#[test]
fn a_complete_value_observation_compares_every_component() {
    let run = fixture_run();
    let comparison = compare_reference(&observation_of(&run), &run);
    assert!(comparison.is_match(), "{:?}", comparison.differences);
    assert!(comparison.unrepresented.is_empty());
    assert_eq!(comparison.compared, BTreeSet::from(ALL_COMPONENTS));
}

#[test]
fn each_state_component_is_named_when_it_differs() {
    let cases: [(PpuReferenceComponent, Mutation); 11] = [
        (PpuReferenceComponent::StateGpr, |_, run| {
            run.observation.state.gpr[3] ^= 1;
        }),
        (PpuReferenceComponent::StateFpr, |_, run| {
            run.observation.state.fpr[31] = 1;
        }),
        (PpuReferenceComponent::StateVr, |_, run| {
            run.observation.state.vr[0] = 1;
        }),
        (PpuReferenceComponent::StatePc, |_, run| {
            run.observation.state.pc += 4;
        }),
        (PpuReferenceComponent::StateCr, |_, run| {
            run.observation.state.cr ^= 1;
        }),
        (PpuReferenceComponent::StateLr, |_, run| {
            run.observation.state.lr ^= 1;
        }),
        (PpuReferenceComponent::StateCtr, |_, run| {
            run.observation.state.ctr ^= 1;
        }),
        (PpuReferenceComponent::StateXer, |_, run| {
            run.observation.state.xer ^= 1;
        }),
        (PpuReferenceComponent::StateVrsave, |_, run| {
            run.observation.state.vrsave ^= 1;
        }),
        (PpuReferenceComponent::StateTb, |_, run| {
            run.observation.state.tb ^= 1;
        }),
        (PpuReferenceComponent::StateReservation, |_, run| {
            run.observation.state.reservation = Some(ReservedLine::containing(0x80));
        }),
    ];
    for (component, mutate) in cases {
        single_named_difference(component, mutate);
    }
}

#[test]
fn each_stop_component_is_named_when_it_differs() {
    let cases: [(PpuReferenceComponent, Mutation); 8] = [
        (PpuReferenceComponent::StopReason, |_, run| {
            run.stop.reason = YieldReason::Finished;
        }),
        (PpuReferenceComponent::StopFault, |_, run| {
            run.stop.fault = Some(FaultKind::Validation);
        }),
        (PpuReferenceComponent::StopPc, |_, run| {
            run.stop.pc = Some(run.stop.pc.map_or(0, |pc| pc + 4));
        }),
        (PpuReferenceComponent::StopLr, |_, run| {
            run.stop.diagnostics.lr = Some(8);
        }),
        (PpuReferenceComponent::StopSyscallLev, |_, run| {
            run.stop.diagnostics.syscall_lev = Some(1);
        }),
        (PpuReferenceComponent::StopFaultingEa, |_, run| {
            run.stop.diagnostics.faulting_ea = Some(0x10);
        }),
        (PpuReferenceComponent::StopFaultRegisters, |_, run| {
            run.stop.diagnostics.fault_regs = Some(FaultRegisterDump {
                gprs: [0; 32],
                lr: 0,
                ctr: 0,
                xer: 0,
                cr: 0,
            });
        }),
        (PpuReferenceComponent::StopSyscallArgs, |_, run| {
            run.stop.syscall_args = Some([1; 9]);
        }),
    ];
    for (component, mutate) in cases {
        single_named_difference(component, mutate);
    }
}

#[test]
fn each_remaining_component_is_named_when_it_differs() {
    let cases: [(PpuReferenceComponent, Mutation); 8] = [
        (PpuReferenceComponent::Memory, |_, run| {
            run.observation.memory[0] ^= 1;
        }),
        (PpuReferenceComponent::Retired, |_, run| {
            run.retired += 1;
        }),
        (PpuReferenceComponent::StagedEffects, |_, run| {
            run.observation.staged_effects.push(Effect::TraceMarker {
                marker: 1,
                source: UnitId::new(0),
            });
        }),
        (PpuReferenceComponent::CommittedEffects, |_, run| {
            run.observation.committed_effects.push(Effect::TraceMarker {
                marker: 1,
                source: UnitId::new(0),
            });
        }),
        (PpuReferenceComponent::Reservations, |_, run| {
            run.observation
                .reservations
                .push((UnitId::new(0), ReservedLine::containing(0x80)));
        }),
        (PpuReferenceComponent::StoreBuffer, |expected, _| {
            expected.store_buffer = value(vec!["pending".into()]);
        }),
        (PpuReferenceComponent::CommitError, |expected, _| {
            expected.commit_error = value(Some("refused".into()));
        }),
        (PpuReferenceComponent::FaultDiscarded, |_, run| {
            run.observation.fault_discarded = !run.observation.fault_discarded;
        }),
    ];
    for (component, mutate) in cases {
        single_named_difference(component, mutate);
    }
}

#[test]
fn a_difference_renders_both_sides_as_debug_text() {
    let difference = single_named_difference(PpuReferenceComponent::StatePc, |expected, run| {
        expected.state.pc = value(4);
        run.observation.state.pc = 8;
    });
    assert_eq!(
        (difference.expected.as_str(), difference.observed.as_str()),
        ("4", "8")
    );

    let difference = single_named_difference(PpuReferenceComponent::StopFault, |expected, run| {
        expected.stop.fault = value(PpuReferenceFault::Guest { code: 7 });
        run.stop.fault = None;
    });
    assert_eq!(difference.expected, "Guest { code: 7 }");
    assert_eq!(difference.observed, "None");

    let difference =
        single_named_difference(PpuReferenceComponent::Reservations, |expected, run| {
            expected.reservations = value(vec![(3, 0x100)]);
            run.observation
                .reservations
                .push((UnitId::new(3), ReservedLine::containing(0x1ff)));
        });
    assert_eq!(difference.expected, "[(3, 256)]");
    assert_eq!(difference.observed, "[(3, 384)]");

    let difference = single_named_difference(PpuReferenceComponent::StateVr, |expected, run| {
        let mut bank = vec![hex_vector(0); 32];
        bank[2] = hex_vector(0xff);
        expected.state.vr_hex = value(bank);
        run.observation.state.vr = [0; 32];
        run.observation.state.vr[2] = 0xf0;
    });
    assert!(difference
        .expected
        .contains("000000000000000000000000000000ff"));
    assert!(difference
        .observed
        .contains("000000000000000000000000000000f0"));
}

#[test]
fn yield_reasons_map_to_their_reference_names() {
    let cases = [
        (
            YieldReason::BudgetExhausted,
            PpuReferenceYieldReason::BudgetExhausted,
        ),
        (
            YieldReason::MailboxAccess,
            PpuReferenceYieldReason::MailboxAccess,
        ),
        (
            YieldReason::DmaSubmitted,
            PpuReferenceYieldReason::DmaSubmitted,
        ),
        (YieldReason::DmaWait, PpuReferenceYieldReason::DmaWait),
        (
            YieldReason::WaitingSync,
            PpuReferenceYieldReason::WaitingSync,
        ),
        (YieldReason::Syscall, PpuReferenceYieldReason::Syscall),
        (
            YieldReason::InterruptBoundary,
            PpuReferenceYieldReason::InterruptBoundary,
        ),
        (YieldReason::Fault, PpuReferenceYieldReason::Fault),
        (YieldReason::Finished, PpuReferenceYieldReason::Finished),
    ];
    let mut run = fixture_run();
    let mut expected = observation_of(&run);
    for (runtime, reference) in cases {
        run.stop.reason = runtime;
        expected.stop.reason = value(reference);
        let comparison = compare_reference(&expected, &run);
        assert!(
            comparison.is_match(),
            "{runtime:?}: {:?}",
            comparison.differences
        );
        for (other_runtime, _) in cases {
            if other_runtime != runtime {
                run.stop.reason = other_runtime;
                let differences = compare_reference(&expected, &run).differences;
                assert_eq!(differences.len(), 1, "{runtime:?} vs {other_runtime:?}");
                assert_eq!(differences[0].field, PpuReferenceComponent::StopReason);
            }
        }
    }
}

#[test]
fn fault_kinds_map_to_reference_faults() {
    let cases = [
        (None, PpuReferenceFault::None),
        (Some(FaultKind::Validation), PpuReferenceFault::Validation),
        (
            Some(FaultKind::Guest(9)),
            PpuReferenceFault::Guest { code: 9 },
        ),
    ];
    let mut run = fixture_run();
    let mut expected = observation_of(&run);
    for (runtime, reference) in cases {
        run.stop.fault = runtime;
        expected.stop.fault = value(reference);
        assert!(compare_reference(&expected, &run).is_match(), "{runtime:?}");
        for (other, _) in cases {
            if other != runtime {
                run.stop.fault = other;
                let differences = compare_reference(&expected, &run).differences;
                assert_eq!(differences.len(), 1, "{runtime:?} vs {other:?}");
                assert_eq!(differences[0].field, PpuReferenceComponent::StopFault);
            }
        }
    }
    expected.stop.fault = value(PpuReferenceFault::Guest { code: 8 });
    run.stop.fault = Some(FaultKind::Guest(9));
    let differences = compare_reference(&expected, &run).differences;
    assert_eq!(differences.len(), 1);
    assert_eq!(differences[0].field, PpuReferenceComponent::StopFault);
}

#[test]
fn fault_registers_and_syscall_args_map_from_the_runtime_dump() {
    let mut run = fixture_run();
    let mut gprs = [0; 32];
    gprs[3] = 7;
    run.stop.diagnostics.fault_regs = Some(FaultRegisterDump {
        gprs,
        lr: 1,
        ctr: 2,
        xer: 3,
        cr: 4,
    });
    run.stop.syscall_args = Some([1, 2, 3, 4, 5, 6, 7, 8, 9]);
    let mut expected = observation_of(&run);
    let registers = PpuReferenceFaultRegisters {
        gpr: gprs.to_vec(),
        lr: 1,
        ctr: 2,
        xer: 3,
        cr: 4,
    };
    expected.stop.fault_registers = value(Some(registers.clone()));
    expected.stop.syscall_args = value(Some((1..=9).collect()));
    assert!(compare_reference(&expected, &run).is_match());

    for (name, swapped) in [
        (
            "lr",
            PpuReferenceFaultRegisters {
                lr: 2,
                ..registers.clone()
            },
        ),
        (
            "ctr",
            PpuReferenceFaultRegisters {
                ctr: 1,
                ..registers.clone()
            },
        ),
        (
            "xer",
            PpuReferenceFaultRegisters {
                xer: 4,
                ..registers.clone()
            },
        ),
        (
            "cr",
            PpuReferenceFaultRegisters {
                cr: 3,
                ..registers.clone()
            },
        ),
        (
            "gpr",
            PpuReferenceFaultRegisters {
                gpr: vec![0; 32],
                ..registers.clone()
            },
        ),
    ] {
        let mut swapped_expected = expected.clone();
        swapped_expected.stop.fault_registers = value(Some(swapped));
        let differences = compare_reference(&swapped_expected, &run).differences;
        assert_eq!(differences.len(), 1, "{name}");
        assert_eq!(
            differences[0].field,
            PpuReferenceComponent::StopFaultRegisters,
            "{name}"
        );
    }
    expected.stop.syscall_args = value(Some((1..=9).rev().collect()));
    let differences = compare_reference(&expected, &run).differences;
    assert_eq!(differences.len(), 1);
    assert_eq!(differences[0].field, PpuReferenceComponent::StopSyscallArgs);
}

#[test]
fn omitted_fields_carry_their_status_and_reason_and_never_differ() {
    let mut run = fixture_run();
    let mut expected = observation_of(&run);
    expected.state.cr = ReferenceField::Undefined {
        reason: "the vector does not constrain CR".into(),
    };
    expected.memory = ReferenceField::Unsupported {
        reason: "the source has no memory image".into(),
    };
    run.observation.state.cr ^= 0xf;
    run.observation.memory[0] ^= 0xff;
    let comparison = compare_reference(&expected, &run);
    assert!(comparison.is_match());
    assert_eq!(comparison.compared.len(), ALL_COMPONENTS.len() - 2);
    assert!(!comparison
        .compared
        .contains(&PpuReferenceComponent::StateCr));
    assert!(!comparison.compared.contains(&PpuReferenceComponent::Memory));
    assert_eq!(
        comparison.unrepresented,
        vec![
            PpuUnrepresentedField {
                field: PpuReferenceComponent::StateCr,
                status: PpuReferenceFieldStatus::Undefined,
                reason: "the vector does not constrain CR".into(),
            },
            PpuUnrepresentedField {
                field: PpuReferenceComponent::Memory,
                status: PpuReferenceFieldStatus::Unsupported,
                reason: "the source has no memory image".into(),
            },
        ]
    );
}

#[test]
fn is_match_ignores_unrepresented_fields_but_not_differences() {
    let omitted = PpuUnrepresentedField {
        field: PpuReferenceComponent::StateTb,
        status: PpuReferenceFieldStatus::Undefined,
        reason: "unconstrained".into(),
    };
    let comparison = PpuReferenceComparison {
        compared: BTreeSet::new(),
        differences: Vec::new(),
        unrepresented: vec![omitted.clone()],
    };
    assert!(comparison.is_match());
    let comparison = PpuReferenceComparison {
        compared: BTreeSet::from([PpuReferenceComponent::StatePc]),
        differences: vec![PpuReferenceDifference {
            field: PpuReferenceComponent::StatePc,
            expected: "4".into(),
            observed: "8".into(),
        }],
        unrepresented: vec![omitted],
    };
    assert!(!comparison.is_match());
}

#[test]
fn replay_of_the_committed_fixture_compares_eighteen_fields_on_four_paths() {
    let replay = replay_reference(&artifact()).expect("fixture replays");
    assert!(replay.internal_divergence.is_none());
    assert_eq!(replay.runs.len(), 4);
    assert_eq!(replay.comparisons.len(), 4);
    for comparison in &replay.comparisons {
        assert!(comparison.is_match(), "{:?}", comparison.differences);
        assert_eq!(comparison.compared.len(), 18);
        assert_eq!(comparison.unrepresented.len(), 9);
    }
    assert_eq!(replay.runs[0].observation.state.gpr[3], 7);
    assert_eq!(replay.runs[0].retired, 1);
}

#[test]
fn replay_names_the_mismatch_of_an_altered_fixture() {
    let mut altered = artifact();
    let mut gpr = vec![0; 32];
    gpr[3] = 8;
    altered.expected.state.gpr = value(gpr);
    let replay = replay_reference(&altered).expect("altered expectation still replays");
    assert!(replay.internal_divergence.is_none());
    for comparison in &replay.comparisons {
        assert!(!comparison.is_match());
        assert_eq!(comparison.differences.len(), 1);
        let difference = &comparison.differences[0];
        assert_eq!(difference.field, PpuReferenceComponent::StateGpr);
        assert!(difference.expected.starts_with("[0, 0, 0, 8, "));
        assert!(difference.observed.starts_with("[0, 0, 0, 7, "));
    }

    let mut altered = artifact();
    altered.expected.retired = value(2);
    let replay = replay_reference(&altered).expect("altered count still replays");
    for comparison in &replay.comparisons {
        assert_eq!(comparison.differences.len(), 1);
        assert_eq!(
            comparison.differences[0].field,
            PpuReferenceComponent::Retired
        );
        assert_eq!(comparison.differences[0].expected, "2");
        assert_eq!(comparison.differences[0].observed, "1");
    }
}

#[test]
fn replay_validates_the_artifact_before_running() {
    let mut altered = artifact();
    altered.schema_version = 2;
    assert!(matches!(
        replay_reference(&altered),
        Err(PpuReferenceError::Version {
            found: 2,
            supported: 1
        })
    ));
    let mut altered = artifact();
    altered.initial_state.gpr.insert(40, 1);
    assert!(matches!(
        replay_reference(&altered),
        Err(PpuReferenceError::RegisterIndex {
            bank: "GPR",
            index: 40
        })
    ));
}

#[test]
fn replay_refuses_an_empty_sequence_or_data_region() {
    let mut altered = artifact();
    altered.words.clear();
    let error = replay_reference(&altered).expect_err("no words");
    assert!(matches!(
        error,
        PpuReferenceError::Replay(crate::ppu_paths::PpuPathError::EmptySequence)
    ));
    let mut altered = artifact();
    altered.initial_memory.clear();
    let error = replay_reference(&altered).expect_err("no data");
    assert!(matches!(
        error,
        PpuReferenceError::Replay(crate::ppu_paths::PpuPathError::EmptyData)
    ));
    assert_eq!(
        error.to_string(),
        "PPU reference replay failed: PPU path data region must contain at least one byte"
    );
}

#[test]
fn initial_state_overrides_reach_every_replayed_path() {
    let mut altered = artifact();
    altered.initial_state.gpr.insert(5, 0x55);
    altered.initial_state.fpr.insert(1, 0x3ff0_0000_0000_0000);
    altered
        .initial_state
        .vr_hex
        .insert(2, hex_vector(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10));
    altered.initial_state.cr = 0x8000_0000;
    altered.initial_state.lr = 0x100;
    altered.initial_state.ctr = 3;
    altered.initial_state.xer = 0x2000_0000;
    altered.initial_state.vrsave = 0xffff_ffff;
    altered.initial_state.reservation = Some(0x80);
    altered.expected.state.cr = ReferenceField::Unsupported {
        reason: "checked directly".into(),
    };
    altered.expected.state.lr = ReferenceField::Unsupported {
        reason: "checked directly".into(),
    };
    altered.expected.state.ctr = ReferenceField::Unsupported {
        reason: "checked directly".into(),
    };
    altered.expected.state.xer = ReferenceField::Unsupported {
        reason: "checked directly".into(),
    };
    altered.expected.state.reservation = ReferenceField::Unsupported {
        reason: "checked directly".into(),
    };
    let mut gpr = vec![0; 32];
    gpr[3] = 7;
    gpr[5] = 0x55;
    altered.expected.state.gpr = value(gpr);
    altered.expected.reservations = value(vec![(0, 0x80)]);
    let replay = replay_reference(&altered).expect("overrides replay");
    assert_eq!(replay.runs.len(), 4);
    for run in &replay.runs {
        let state = &run.observation.state;
        assert_eq!(state.gpr[5], 0x55);
        assert_eq!(state.fpr[1], 0x3ff0_0000_0000_0000);
        assert_eq!(state.vr[2], 0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10);
        assert_eq!(state.cr, 0x8000_0000);
        assert_eq!(state.lr, 0x100);
        assert_eq!(state.ctr, 3);
        assert_eq!(state.xer, 0x2000_0000);
        assert_eq!(state.vrsave, 0xffff_ffff);
        assert_eq!(state.reservation, Some(ReservedLine::containing(0x80)));
    }
    for comparison in &replay.comparisons {
        assert!(comparison.is_match(), "{:?}", comparison.differences);
    }
}

#[test]
fn to_state_refuses_a_vector_override_it_cannot_parse() {
    let mut input = artifact().initial_state;
    input.vr_hex.insert(4, "zz".into());
    assert!(matches!(
        input.to_state(),
        Err(PpuReferenceError::VectorValue { index: 4 })
    ));
    input.vr_hex.clear();
    input.gpr.insert(32, 1);
    assert!(matches!(
        input.to_state(),
        Err(PpuReferenceError::RegisterIndex {
            bank: "GPR",
            index: 32
        })
    ));
}
