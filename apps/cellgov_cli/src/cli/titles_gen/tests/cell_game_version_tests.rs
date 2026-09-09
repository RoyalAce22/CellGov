use cellgov_compare::{AppVersion, BootOutcome};

use super::super::test_fixtures::*;
use super::*;

/// An anchor stamped with both halves of the cell it was measured
/// at; `game` takes the identity spelling of the game version.
fn boot_at(fw: &str, game: Option<&str>) -> BootSummary {
    let mut b = boot(BootOutcome::ProcessExit, 1_000);
    b.identity = RunIdentity {
        firmware: Some(firmware(fw)),
        game: game.map(|v| GameIdentity {
            title_id: "NPAA61000".to_string(),
            version: v.to_string(),
            app_version: Some(AppVersion::AppVer("01.00".to_string())),
        }),
    };
    b
}

#[test]
fn an_anchor_measured_against_another_game_version_does_not_answer_for_this_cell() {
    let fixtures = Fixtures::new("gamever-misfiled");
    let t = title("NPAA61000", "Misfiled Version", 2008, "Studio");
    fixtures.write_anchor(
        "NPAA61000",
        &reference_key(),
        &boot_at(REFERENCE_FW, Some("update:02.51")),
    );
    match load_title(&t, fixtures.path()) {
        Err(SummaryLoadError::CellGameVersionMismatch { cell, recorded, .. }) => {
            assert_eq!((cell.as_str(), recorded.as_str()), (BASE, "update:02.51"));
        }
        other => panic!("expected a game-version refusal, got {other:?}"),
    }
}

#[test]
fn an_update_cell_accepts_the_identitys_update_spelling() {
    let fixtures = Fixtures::new("gamever-update");
    let key = cell_key(REFERENCE_FW, Some("02.51"));
    let mut t = title("NPAA61001", "Updated", 2008, "Studio");
    t.matrix.push(matrix_cell(key.clone()));
    fixtures.write_anchor(
        "NPAA61001",
        &key,
        &boot_at(REFERENCE_FW, Some("update:02.51")),
    );
    let docs = load_title(&t, fixtures.path()).unwrap();
    let update = docs.cells.iter().find(|c| c.key == key).unwrap();
    assert!(update.artifacts.boot.is_some());
}

#[test]
fn a_firmware_shipped_cell_refuses_an_anchor_naming_a_store_entry() {
    let fixtures = Fixtures::new("gamever-firmware-exec");
    let key = cell_key(REFERENCE_FW, None);
    let t = firmware_exec_title("VSHVER", "Firmware Exec", &[REFERENCE_FW]);
    fixtures.write_anchor("VSHVER", &key, &boot_at(REFERENCE_FW, Some(BASE)));
    assert!(matches!(
        load_title(&t, fixtures.path()),
        Err(SummaryLoadError::CellGameVersionMismatch { .. })
    ));
}

#[test]
fn an_anchor_naming_no_store_entry_still_loads() {
    let fixtures = Fixtures::new("gamever-unstamped");
    let t = title("NPAA61002", "Unstamped Version", 2008, "Studio");
    fixtures.write_anchor("NPAA61002", &reference_key(), &boot_at(REFERENCE_FW, None));
    assert!(load_title(&t, fixtures.path())
        .unwrap()
        .reference()
        .unwrap()
        .artifacts
        .boot
        .is_some());
}
