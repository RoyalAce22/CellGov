//! A committed file that disagrees with its cell on several axes is
//! refused for the firmware first, then the game version, then the
//! boot overrides.

use std::path::Path;

use cellgov_compare::{BootOverrides, GameIdentity};

use super::super::test_fixtures::*;
use super::*;

fn disagreeing(fw: &str, game: &str, overridden: bool) -> RunIdentity {
    RunIdentity {
        firmware: Some(firmware(fw)),
        game: Some(GameIdentity {
            title_id: "NPAA61100".to_string(),
            version: game.to_string(),
            app_version: None,
        }),
        overrides: BootOverrides {
            force_system_authid: overridden,
            ..BootOverrides::default()
        },
    }
}

#[test]
fn the_firmware_is_named_before_the_game_version_and_the_overrides() {
    let found = check_cell(
        Path::new("anchor.json"),
        &reference_key(),
        &disagreeing("2.76", "update:02.51", true),
    );
    assert!(
        matches!(
            &found,
            Err(SummaryLoadError::CellFirmwareMismatch { recorded, .. }) if recorded == "2.76"
        ),
        "{found:?}"
    );
}

#[test]
fn the_game_version_is_named_before_the_overrides() {
    let found = check_cell(
        Path::new("anchor.json"),
        &reference_key(),
        &disagreeing(REFERENCE_FW, "update:02.51", true),
    );
    assert!(
        matches!(
            &found,
            Err(SummaryLoadError::CellGameVersionMismatch { recorded, .. })
                if recorded == "update:02.51"
        ),
        "{found:?}"
    );
}
