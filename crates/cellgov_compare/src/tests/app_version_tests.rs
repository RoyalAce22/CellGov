//! The game half's version: which PARAM.SFO key named it, on the wire
//! and in the fingerprint.

use super::*;

fn game(app_version: Option<AppVersion>) -> GameIdentity {
    GameIdentity {
        title_id: "NPAA00001".into(),
        version: "base".into(),
        app_version,
    }
}

fn wire(id: &GameIdentity) -> serde_json::Value {
    serde_json::to_value(id).expect("serialize")
}

#[test]
fn an_app_ver_writes_the_app_ver_key_alone() {
    let json = wire(&game(Some(AppVersion::AppVer("01.00".into()))));
    assert_eq!(json["app_ver"], "01.00");
    assert!(json.get("sfo_version").is_none(), "{json}");
}

#[test]
fn a_version_standing_in_writes_the_sfo_version_key_alone() {
    let json = wire(&game(Some(AppVersion::SfoVersion("01.02".into()))));
    assert_eq!(json["sfo_version"], "01.02");
    assert!(json.get("app_ver").is_none(), "{json}");
}

#[test]
fn a_tree_naming_no_version_writes_neither_key() {
    let json = wire(&game(None));
    assert!(json.get("app_ver").is_none(), "{json}");
    assert!(json.get("sfo_version").is_none(), "{json}");
    assert_eq!(json["title_id"], "NPAA00001");
}

/// `GameIdentityWire` spells the two keys as serde field names;
/// `AppVersion::key` spells them as literals. The label and the
/// fingerprint use the literals, so the two spellings must agree.
#[test]
fn the_key_label_is_the_wire_key() {
    for version in [
        AppVersion::AppVer("01.00".into()),
        AppVersion::SfoVersion("01.00".into()),
    ] {
        let json = wire(&game(Some(version.clone())));
        assert_eq!(json[version.key()], "01.00", "{json}");
    }
}

#[test]
fn every_shape_reads_back() {
    for id in [
        game(Some(AppVersion::AppVer("01.00".into()))),
        game(Some(AppVersion::SfoVersion("01.02".into()))),
        game(None),
    ] {
        let back: GameIdentity = serde_json::from_value(wire(&id)).expect("deserialize");
        assert_eq!(back, id);
    }
}

#[test]
fn a_recorded_identity_from_before_the_key_was_named_still_reads() {
    let text = r#"{"title_id": "NPAA00001", "version": "base", "app_ver": "01.00"}"#;
    let back: GameIdentity = serde_json::from_str(text).expect("deserialize");
    assert_eq!(back.app_version, Some(AppVersion::AppVer("01.00".into())));
}

#[test]
fn both_keys_at_once_are_refused() {
    let text = r#"{"title_id": "NPAA00001", "version": "base", "app_ver": "01.00", "sfo_version": "01.00"}"#;
    let err = serde_json::from_str::<GameIdentity>(text).unwrap_err();
    assert!(err.to_string().contains("both"), "{err}");
}

#[test]
fn a_key_the_wire_does_not_own_is_refused() {
    let text = r#"{"title_id": "NPAA00001", "version": "base", "app_ver": "01.00", "extra": 1}"#;
    let err = serde_json::from_str::<GameIdentity>(text).unwrap_err();
    assert!(
        err.to_string().contains("unknown field `extra`"),
        "the refusal names the key: {err}"
    );
}

#[test]
fn one_string_under_the_two_keys_fingerprints_apart() {
    let by = |app_version| {
        RunIdentity {
            firmware: None,
            game: Some(game(app_version)),
        }
        .game_fingerprint()
    };
    let app_ver = by(Some(AppVersion::AppVer("01.00".into())));
    let sfo_version = by(Some(AppVersion::SfoVersion("01.00".into())));
    let none = by(None);
    assert_ne!(app_ver, sfo_version);
    assert_ne!(app_ver, none);
    assert_ne!(sfo_version, none);
}

#[test]
fn the_label_names_the_key_or_its_absence() {
    assert_eq!(
        game(Some(AppVersion::AppVer("01.00".into()))).app_version_label(),
        "app_ver 01.00"
    );
    assert_eq!(
        game(Some(AppVersion::SfoVersion("01.02".into()))).app_version_label(),
        "sfo_version 01.02"
    );
    assert_eq!(game(None).app_version_label(), "no version key");
}

#[test]
fn a_difference_in_the_version_key_alone_is_named_in_the_warning() {
    let by = |app_version| RunIdentity {
        firmware: None,
        game: Some(game(app_version)),
    };
    let a = by(Some(AppVersion::AppVer("01.00".into())));
    let b = by(Some(AppVersion::SfoVersion("01.00".into())));
    let lines = cross_identity_warning(&a, "a.json", &b, "b.json");
    let warning = lines
        .iter()
        .find(|l| l.contains("cross-version comparison"))
        .unwrap_or_else(|| panic!("the game halves differ: {lines:?}"));
    assert!(
        warning.contains("(app_ver 01.00)") && warning.contains("(sfo_version 01.00)"),
        "each side names its key: {warning}"
    );
}

#[test]
fn a_tree_naming_no_version_is_named_as_such_in_the_warning() {
    let by = |app_version| RunIdentity {
        firmware: None,
        game: Some(game(app_version)),
    };
    let a = by(Some(AppVersion::AppVer("01.00".into())));
    let b = by(None);
    let lines = cross_identity_warning(&a, "a.json", &b, "b.json");
    assert!(
        lines
            .iter()
            .any(|l| l.contains("(app_ver 01.00)") && l.contains("(no version key)")),
        "{lines:?}"
    );
}

#[test]
fn the_report_line_carries_the_label() {
    let id = RunIdentity {
        firmware: None,
        game: Some(game(Some(AppVersion::SfoVersion("01.02".into())))),
    };
    let lines = id.render_lines();
    assert!(lines[0].contains("(sfo_version 01.02)"), "{lines:?}");
}
