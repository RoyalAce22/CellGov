//! `dev record-anchors` refuses a measurement taken under a boot
//! override.

use super::*;
use cellgov_compare::{BootOverrides, FirmwareIdentity, GameIdentity};
use cellgov_install::store::BASE_GAME_VER;

fn cell() -> CellKey {
    CellKey {
        fw: "4.93".to_string(),
        game_ver: Some(BASE_GAME_VER.to_string()),
    }
}

fn identity_under(overrides: BootOverrides) -> RunIdentity {
    RunIdentity {
        firmware: Some(FirmwareIdentity {
            version: "4.93".to_string(),
            image_version: "0000000000000000".to_string(),
            pup_sha256: "0".repeat(64),
        }),
        game: Some(GameIdentity {
            title_id: "CG_TEST".to_string(),
            version: BASE_GAME_VER.to_string(),
            app_version: None,
        }),
        overrides,
    }
}

#[test]
fn a_run_under_no_override_names_its_cell() {
    assert!(cell_disagreements(&identity_under(BootOverrides::default()), &cell()).is_empty());
}

#[test]
fn a_run_under_an_override_is_refused_even_at_its_own_cell() {
    let found = cell_disagreements(
        &identity_under(BootOverrides {
            skip_module_start: true,
            prx_base: Some(0x3000_0000),
            ..BootOverrides::default()
        }),
        &cell(),
    );
    assert_eq!(
        found.len(),
        1,
        "the cell matches, so the override set is the one disagreement: {found:?}"
    );
    assert!(
        found[0].contains("skip_module_start") && found[0].contains("prx_base=0x30000000"),
        "{found:?}"
    );
}
