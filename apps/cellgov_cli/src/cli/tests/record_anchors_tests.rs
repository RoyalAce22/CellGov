use super::*;

fn key(fw: &str, game_ver: Option<&str>) -> CellKey {
    CellKey {
        fw: fw.to_string(),
        game_ver: game_ver.map(str::to_string),
    }
}

/// A `RUN_IDENTITY` payload naming one firmware and one game version.
fn identity(fw: Option<&str>, game_ver: Option<&str>) -> RunIdentity {
    RunIdentity {
        firmware: fw.map(|v| cellgov_compare::FirmwareIdentity {
            version: v.to_string(),
            image_version: "0000000000000000".to_string(),
            pup_sha256: "0".repeat(64),
        }),
        game: game_ver.map(|v| cellgov_compare::GameIdentity {
            title_id: "CG_TEST".to_string(),
            version: v.to_string(),
            app_version: Some(cellgov_compare::AppVersion::AppVer("01.00".to_string())),
        }),
        overrides: Default::default(),
    }
}

#[test]
fn an_identity_naming_the_cell_disagrees_about_nothing() {
    assert!(cell_disagreements(
        &identity(Some("4.93"), Some("base")),
        &key("4.93", Some("base"))
    )
    .is_empty());
    assert!(cell_disagreements(
        &identity(Some("4.93"), Some("update:02.51")),
        &key("4.93", Some("02.51"))
    )
    .is_empty());
    assert!(
        cell_disagreements(&identity(Some("4.93"), None), &key("4.93", None)).is_empty(),
        "a firmware-shipped title has no game-version axis to disagree about"
    );
}

#[test]
fn a_firmware_free_run_disagrees_with_the_cell_it_was_asked_for() {
    let found = cell_disagreements(&identity(None, Some("base")), &key("4.93", Some("base")));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("no managed firmware"), "{found:?}");
    assert!(found[0].contains("4.93"), "{found:?}");
}

#[test]
fn a_run_at_another_firmware_or_game_version_is_named_per_axis() {
    let found = cell_disagreements(
        &identity(Some("3.55"), Some("update:02.51")),
        &key("4.93", Some("base")),
    );
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found[0].contains("3.55") && found[0].contains("4.93"),
        "{found:?}"
    );
    assert!(
        found[1].contains("update:02.51") && found[1].contains("base"),
        "{found:?}"
    );
}

#[test]
fn an_unnamed_game_half_disagrees_with_a_cell_that_names_one() {
    let found = cell_disagreements(&identity(Some("4.93"), None), &key("4.93", Some("base")));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("(none)") && found[0].contains("base"),
        "{found:?}"
    );
}
