//! Which firmware each runner ran, and when that stops being a
//! parity verdict.

use super::*;
use crate::identity::{AppVersion, FirmwareIdentity, GameIdentity};

fn equivalent() -> CrossRunnerSummary {
    CrossRunnerSummary {
        convergence: Convergence::Yes,
        byte_parity: ByteParity::Equivalent,
        per_class_bytes: BTreeMap::new(),
        unclassified_bytes: 0,
        unclassified_runs: Vec::new(),
        lowest_offset_class: None,
        identity: RunIdentity::default(),
        rpcs3_firmware: None,
        oracle_gap_ordinals: None,
    }
}

fn identity(fw: &str) -> RunIdentity {
    RunIdentity {
        firmware: Some(FirmwareIdentity {
            version: fw.to_string(),
            image_version: "0x0000000000010b94".to_string(),
            pup_sha256: "ab".to_string(),
        }),
        game: Some(GameIdentity {
            title_id: "NPUA80001".to_string(),
            version: "base".to_string(),
            app_version: Some(AppVersion::AppVer("01.00".to_string())),
        }),
        overrides: Default::default(),
    }
}

fn json_of(summary: &CrossRunnerSummary) -> serde_json::Value {
    serde_json::to_value(summary).unwrap()
}

#[test]
fn one_library_on_both_sides_is_a_verdict() {
    let summary = equivalent()
        .with_firmware(identity("4.93"), "4.93".to_string())
        .unwrap();
    summary.validate().unwrap();
    let back: CrossRunnerSummary = serde_json::from_value(json_of(&summary)).unwrap();
    assert_eq!(back, summary);
}

#[test]
fn two_libraries_are_refused_naming_both() {
    let err = equivalent()
        .with_firmware(identity("4.93"), "4.92".to_string())
        .unwrap_err();
    assert_eq!(
        err,
        CrossRunnerSummaryError::FirmwareDisagreement {
            cellgov: "4.93".to_string(),
            rpcs3: "4.92".to_string(),
        }
    );
    let rendered = err.to_string();
    assert!(
        rendered.contains("4.93") && rendered.contains("4.92"),
        "{rendered}"
    );
}

#[test]
fn a_disagreeing_file_fails_to_load() {
    let mut summary = equivalent();
    summary.identity = identity("4.93");
    summary.rpcs3_firmware = Some("3.55".to_string());
    let err = serde_json::from_value::<CrossRunnerSummary>(json_of(&summary)).unwrap_err();
    let rendered = err.to_string();
    assert!(
        rendered.contains("4.93") && rendered.contains("3.55"),
        "{rendered}"
    );
}

#[test]
fn a_stamped_file_missing_the_other_runners_version_is_refused() {
    let mut summary = equivalent();
    summary.identity = identity("4.93");
    let err = serde_json::from_value::<CrossRunnerSummary>(json_of(&summary)).unwrap_err();
    assert!(err.to_string().contains("4.93"), "{err}");
    assert_eq!(
        summary.validate().unwrap_err(),
        CrossRunnerSummaryError::Rpcs3FirmwareUnnamed {
            cellgov: "4.93".to_string(),
        }
    );
}

#[test]
fn a_file_naming_only_the_other_runners_version_is_refused() {
    let mut summary = equivalent();
    summary.rpcs3_firmware = Some("4.92".to_string());
    assert_eq!(
        summary.validate().unwrap_err(),
        CrossRunnerSummaryError::CellgovFirmwareUnnamed {
            rpcs3: "4.92".to_string(),
        }
    );
    assert!(serde_json::from_value::<CrossRunnerSummary>(json_of(&summary)).is_err());
}

#[test]
fn a_summary_naming_neither_side_still_loads() {
    let summary = equivalent();
    let value = json_of(&summary);
    assert!(value.get("firmware").is_none(), "{value}");
    assert!(value.get("rpcs3_firmware").is_none(), "{value}");
    let back: CrossRunnerSummary = serde_json::from_value(value).unwrap();
    assert_eq!(back, summary);
}

#[test]
fn a_stamped_summary_writes_both_versions_at_the_top_level() {
    let summary = equivalent()
        .with_firmware(identity("4.93"), "4.93".to_string())
        .unwrap();
    let value = json_of(&summary);
    assert_eq!(value["firmware"]["version"], "4.93", "{value}");
    assert_eq!(value["game"]["title_id"], "NPUA80001", "{value}");
    assert_eq!(value["rpcs3_firmware"], "4.93", "{value}");
}

#[test]
fn a_stale_key_in_a_summary_file_is_refused() {
    let summary = equivalent()
        .with_firmware(identity("4.93"), "4.93".to_string())
        .unwrap();
    let mut value = json_of(&summary);
    value
        .as_object_mut()
        .expect("a summary serializes as an object")
        .insert("rpcs3_fw".to_string(), serde_json::Value::from("4.92"));
    let err = serde_json::from_value::<CrossRunnerSummary>(value).unwrap_err();
    assert!(err.to_string().contains("rpcs3_fw"), "{err}");
}

#[test]
fn a_title_with_no_version_axis_still_stamps_a_verdict() {
    let mut id = identity("4.93");
    id.game = None;
    let summary = equivalent().with_firmware(id, "4.93".to_string()).unwrap();
    let value = json_of(&summary);
    assert!(value.get("game").is_none(), "{value}");
    let back: CrossRunnerSummary = serde_json::from_value(value).unwrap();
    assert_eq!(back, summary);
}

#[test]
fn an_unmanaged_composition_cannot_stamp_a_verdict() {
    let err = equivalent()
        .with_firmware(RunIdentity::default(), "4.93".to_string())
        .unwrap_err();
    assert_eq!(
        err,
        CrossRunnerSummaryError::CellgovFirmwareUnnamed {
            rpcs3: "4.93".to_string(),
        }
    );
}
