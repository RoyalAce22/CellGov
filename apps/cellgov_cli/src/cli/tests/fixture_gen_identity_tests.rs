use super::capture_identity_refusal;
use cellgov_compare::{AppVersion, FirmwareIdentity, GameIdentity, RunIdentity};

fn identity(firmware: Option<&str>, game: Option<&str>) -> RunIdentity {
    RunIdentity {
        firmware: firmware.map(|version| FirmwareIdentity {
            version: version.to_string(),
            image_version: format!("image-{version}"),
            pup_sha256: format!("sha-{version}"),
        }),
        game: game.map(|version| GameIdentity {
            title_id: "TEST00001".to_string(),
            version: version.to_string(),
            app_version: Some(AppVersion::AppVer("01.00".to_string())),
        }),
        ..RunIdentity::default()
    }
}

#[test]
fn matching_capture_identity_is_accepted() {
    let selected = identity(Some("4.93"), Some("base"));
    assert_eq!(
        capture_identity_refusal("cg.json", &selected, &selected),
        None
    );
}

#[test]
fn absent_capture_halves_remain_compatible() {
    let captured = RunIdentity::default();
    let selected = identity(Some("4.93"), Some("base"));
    assert_eq!(
        capture_identity_refusal("cg.json", &captured, &selected),
        None
    );
}

#[test]
fn a_different_firmware_is_refused_by_name() {
    let captured = identity(Some("4.91"), Some("base"));
    let selected = identity(Some("4.93"), Some("base"));
    let refusal = capture_identity_refusal("cg.json", &captured, &selected)
        .expect("a mismatched firmware is refused");
    assert!(refusal.contains("cg.json"), "{refusal}");
    assert!(refusal.contains("firmware identity"), "{refusal}");
}

#[test]
fn a_different_game_version_is_refused_by_name() {
    let captured = identity(Some("4.93"), Some("base"));
    let selected = identity(Some("4.93"), Some("update:01.01"));
    let refusal = capture_identity_refusal("cg.json", &captured, &selected)
        .expect("a mismatched game is refused");
    assert!(refusal.contains("cg.json"), "{refusal}");
    assert!(refusal.contains("game identity"), "{refusal}");
}
