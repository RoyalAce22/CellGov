//! Literal pins on the three run-identity fingerprints a trace header
//! carries. A change to `fingerprint`'s framing, to its hasher, or to a
//! half's input fields moves one of these values.

use super::*;

const BY_HAND: &str = "written by hand in the same commit as the change that moves it";

fn firmware() -> FirmwareIdentity {
    FirmwareIdentity {
        version: "4.91".into(),
        image_version: "0x0000000000010b82".into(),
        pup_sha256: "ab".repeat(32),
    }
}

fn with_game(version: &str, app_version: Option<AppVersion>) -> RunIdentity {
    RunIdentity {
        firmware: None,
        game: Some(GameIdentity {
            title_id: "NPAA00001".into(),
            version: version.into(),
            app_version,
        }),
        overrides: BootOverrides::default(),
    }
}

#[test]
fn firmware_fingerprint_wire_format_golden() {
    let id = RunIdentity {
        firmware: Some(firmware()),
        game: None,
        overrides: BootOverrides::default(),
    };
    assert_eq!(
        id.firmware_fingerprint(),
        0x108b_c618_95ad_35f4,
        "{BY_HAND}"
    );
}

#[test]
fn an_absent_firmware_half_fingerprints_as_zero() {
    assert_eq!(RunIdentity::default().firmware_fingerprint(), 0);
}

#[test]
fn game_fingerprint_wire_format_golden() {
    let app_ver = with_game("base", Some(AppVersion::AppVer("01.00".into())));
    assert_eq!(
        app_ver.game_fingerprint(),
        0xca99_5968_b4ae_35ba,
        "{BY_HAND}"
    );
    let sfo = with_game("update:01.02", Some(AppVersion::SfoVersion("01.02".into())));
    assert_eq!(sfo.game_fingerprint(), 0x94e9_f4e2_5454_8e1b, "{BY_HAND}");
    let none = with_game("base", None);
    assert_eq!(none.game_fingerprint(), 0x1a30_49f7_24bb_1ce2, "{BY_HAND}");
}

#[test]
fn overrides_fingerprint_wire_format_golden() {
    let id = RunIdentity {
        firmware: Some(firmware()),
        game: None,
        overrides: BootOverrides {
            skip_module_start: true,
            prx_base: Some(0x2000_0000),
            ..BootOverrides::default()
        },
    };
    assert_eq!(
        id.overrides_fingerprint(),
        0x24e6_4ae3_e9df_cb9f,
        "{BY_HAND}"
    );
}
