use cellgov_compare::{BootOverrides, FirmwareIdentity};

use super::*;

fn key(fw: &str, game_ver: Option<&str>) -> CellKey {
    CellKey {
        fw: fw.to_string(),
        game_ver: game_ver.map(str::to_string),
    }
}

fn identity(fw: Option<&str>, game_ver: Option<&str>) -> RunIdentity {
    RunIdentity {
        firmware: fw.map(|v| FirmwareIdentity {
            version: v.to_string(),
            image_version: "0000000000000000".to_string(),
            pup_sha256: "0".repeat(64),
        }),
        game: game_ver.map(|v| GameIdentity {
            title_id: "CG_TEST".to_string(),
            version: v.to_string(),
            app_version: None,
        }),
        overrides: BootOverrides::default(),
    }
}

#[test]
fn an_identity_naming_the_cell_disagrees_about_nothing() {
    assert!(key("4.93", Some("base"))
        .disagreements(&identity(Some("4.93"), Some("base")))
        .is_empty());
    assert!(
        key("4.93", Some("02.51"))
            .disagreements(&identity(Some("4.93"), Some("update:02.51")))
            .is_empty(),
        "an update is compared in the identity's spelling"
    );
    assert!(key("4.93", None)
        .disagreements(&identity(Some("4.93"), None))
        .is_empty());
}

#[test]
fn each_axis_reports_its_own_disagreement_overrides_first() {
    let mut run = identity(Some("3.55"), Some("update:02.51"));
    run.overrides.force_system_authid = true;
    assert_eq!(
        key("4.93", Some("base")).disagreements(&run),
        vec![
            CellDisagreement::Overridden {
                names: vec!["force_system_authid".to_string()],
            },
            CellDisagreement::FirmwareMismatch {
                cell: "4.93".to_string(),
                recorded: "3.55".to_string(),
            },
            CellDisagreement::GameVersionMismatch {
                cell: Some("base".to_string()),
                recorded: "update:02.51".to_string(),
            },
        ]
    );
}

#[test]
fn an_identity_naming_nothing_is_reported_as_absence() {
    assert_eq!(
        key("4.93", Some("base")).disagreements(&identity(None, None)),
        vec![
            CellDisagreement::NoFirmware {
                cell: "4.93".to_string(),
            },
            CellDisagreement::NoGameVersion {
                cell: "base".to_string(),
            },
        ]
    );
}

#[test]
fn a_game_version_on_a_firmware_shipped_cell_is_a_mismatch() {
    assert_eq!(
        key("4.93", None).disagreements(&identity(Some("4.93"), Some("base"))),
        vec![CellDisagreement::GameVersionMismatch {
            cell: None,
            recorded: "base".to_string(),
        }]
    );
}
