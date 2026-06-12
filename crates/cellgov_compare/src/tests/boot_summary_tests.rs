//! BootSummary checkpoint/outcome pairing guards and structural JSON shape per checkpoint kind.

use super::*;

fn round_trip(s: &BootSummary) {
    let json = serde_json::to_string_pretty(s).unwrap();
    let parsed: BootSummary = serde_json::from_str(&json).unwrap();
    assert_eq!(&parsed, s);
}

#[test]
fn round_trip_process_exit() {
    round_trip(
        &BootSummary::new(
            CheckpointKind::ProcessExit,
            BootOutcome::ProcessExit,
            195_312,
            Budget::new(256),
        )
        .unwrap(),
    );
}

#[test]
fn round_trip_first_rsx_write() {
    round_trip(
        &BootSummary::new(
            CheckpointKind::FirstRsxWrite,
            BootOutcome::RsxWriteCheckpoint,
            14_352_589,
            Budget::new(256),
        )
        .unwrap(),
    );
}

#[test]
fn round_trip_pc_payload() {
    round_trip(
        &BootSummary::new(
            CheckpointKind::Pc {
                addr: GuestAddr::new(0x10381ce8),
            },
            BootOutcome::PcReached(0x10381ce8),
            1,
            Budget::new(1),
        )
        .unwrap(),
    );
}

#[test]
fn round_trip_fault() {
    round_trip(
        &BootSummary::new(
            CheckpointKind::FirstRsxWrite,
            BootOutcome::Fault,
            100,
            Budget::new(256),
        )
        .unwrap(),
    );
}

#[test]
fn round_trip_max_steps() {
    round_trip(
        &BootSummary::new(
            CheckpointKind::ProcessExit,
            BootOutcome::MaxSteps,
            500,
            Budget::new(256),
        )
        .unwrap(),
    );
}

#[test]
fn round_trip_time_overflow() {
    round_trip(
        &BootSummary::new(
            CheckpointKind::FirstRsxWrite,
            BootOutcome::TimeOverflow,
            7,
            Budget::new(256),
        )
        .unwrap(),
    );
}

#[test]
fn round_trip_zero_steps() {
    round_trip(
        &BootSummary::new(
            CheckpointKind::Pc {
                addr: GuestAddr::new(0x10381ce8),
            },
            BootOutcome::PcReached(0x10381ce8),
            0,
            Budget::new(1),
        )
        .unwrap(),
    );
}

#[test]
fn round_trip_zero_budget() {
    round_trip(
        &BootSummary::new(
            CheckpointKind::ProcessExit,
            BootOutcome::ProcessExit,
            10,
            Budget::ZERO,
        )
        .unwrap(),
    );
}

#[test]
fn pc_outcome_with_non_pc_checkpoint_rejected() {
    let err = BootSummary::new(
        CheckpointKind::ProcessExit,
        BootOutcome::PcReached(0x10381ce8),
        1,
        Budget::new(1),
    )
    .unwrap_err();
    assert!(
        matches!(err, BootSummaryError::PcReachedWithoutPcCheckpoint { .. }),
        "got {err:?}"
    );
}

#[test]
fn rsx_write_outcome_with_non_rsx_checkpoint_rejected() {
    let err = BootSummary::new(
        CheckpointKind::ProcessExit,
        BootOutcome::RsxWriteCheckpoint,
        1,
        Budget::new(1),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            BootSummaryError::RsxWriteOutcomeWithoutRsxCheckpoint { .. }
        ),
        "got {err:?}"
    );
}

#[test]
fn pc_outcome_address_mismatch_rejected() {
    let err = BootSummary::new(
        CheckpointKind::Pc {
            addr: GuestAddr::new(0x10381ce8),
        },
        BootOutcome::PcReached(0xdeadbeef),
        1,
        Budget::new(1),
    )
    .unwrap_err();
    let BootSummaryError::PcAddressMismatch {
        checkpoint,
        outcome,
    } = err
    else {
        panic!("expected PcAddressMismatch, got {err:?}");
    };
    assert_eq!(checkpoint, GuestAddr::new(0x10381ce8));
    assert_eq!(outcome, GuestAddr::new(0xdeadbeef));
}

#[test]
fn deserialize_rejects_invalid_pair() {
    let json = r#"{
        "checkpoint": { "kind": "process_exit" },
        "outcome": "RsxWriteCheckpoint",
        "steps": 1,
        "budget": 1
    }"#;
    let res: Result<BootSummary, _> = serde_json::from_str(json);
    assert!(
        res.is_err(),
        "deserialize must reject mismatched checkpoint/outcome"
    );
}

#[test]
fn insns_overflow_rejected() {
    let err = BootSummary::new(
        CheckpointKind::ProcessExit,
        BootOutcome::ProcessExit,
        u64::MAX,
        Budget::new(2),
    )
    .unwrap_err();
    assert!(matches!(err, BootSummaryError::InsnsOverflow { .. }));
}

#[test]
fn insns_method_matches_steps_times_budget() {
    let s = BootSummary::new(
        CheckpointKind::FirstRsxWrite,
        BootOutcome::RsxWriteCheckpoint,
        14_352_589,
        Budget::new(256),
    )
    .unwrap();
    assert_eq!(s.insns(), 14_352_589u64 * 256);
}

#[test]
fn json_shape_pc_payload_is_structural() {
    let s = BootSummary::new(
        CheckpointKind::Pc {
            addr: GuestAddr::new(0x10381ce8),
        },
        BootOutcome::PcReached(0x10381ce8),
        1,
        Budget::new(1),
    )
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(v["checkpoint"]["kind"], "pc");
    assert_eq!(v["checkpoint"]["addr"], serde_json::json!(0x10381ce8_u64));
    assert_eq!(
        v["outcome"],
        serde_json::json!({ "PcReached": 0x10381ce8_u64 })
    );
    assert_eq!(v["steps"], serde_json::json!(1u64));
    assert_eq!(v["budget"], serde_json::json!(1u64));
    assert!(v.get("insns").is_none(), "insns is not a serialized field");
}

#[test]
fn json_shape_process_exit_is_structural() {
    let s = BootSummary::new(
        CheckpointKind::ProcessExit,
        BootOutcome::ProcessExit,
        195_312,
        Budget::new(256),
    )
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(v["checkpoint"]["kind"], "process_exit");
    assert_eq!(v["outcome"], "ProcessExit");
    assert_eq!(v["steps"], serde_json::json!(195_312u64));
    assert_eq!(v["budget"], serde_json::json!(256u64));
}

#[test]
fn json_shape_first_rsx_write_is_structural() {
    let s = BootSummary::new(
        CheckpointKind::FirstRsxWrite,
        BootOutcome::RsxWriteCheckpoint,
        14_352_589,
        Budget::new(256),
    )
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(v["checkpoint"]["kind"], "first_rsx_write");
    assert_eq!(v["outcome"], "RsxWriteCheckpoint");
    assert_eq!(v["steps"], serde_json::json!(14_352_589u64));
    assert_eq!(v["budget"], serde_json::json!(256u64));
}

#[test]
fn checkpoint_kind_variant_json_keys_are_stable() {
    let pe = serde_json::to_value(CheckpointKind::ProcessExit).unwrap();
    let rsx = serde_json::to_value(CheckpointKind::FirstRsxWrite).unwrap();
    let pc = serde_json::to_value(CheckpointKind::Pc {
        addr: GuestAddr::new(0x1234),
    })
    .unwrap();
    assert_eq!(pe["kind"], "process_exit");
    assert_eq!(rsx["kind"], "first_rsx_write");
    assert_eq!(pc["kind"], "pc");
    assert_eq!(pc["addr"], serde_json::json!(0x1234u64));
}
