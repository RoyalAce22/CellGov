use super::*;

use std::path::PathBuf;

#[test]
fn a_store_path_renders_relative_and_slash_separated() {
    let root = PathBuf::from("vfs");
    let path = root.join("titles").join("TEST00000").join("base");
    assert_eq!(store_rel(&root, &path), "titles/TEST00000/base");
}

#[test]
fn the_root_itself_renders_as_the_empty_relative_path() {
    let root = PathBuf::from("vfs");
    assert_eq!(store_rel(&root, &root), "");
}

#[test]
fn a_path_outside_the_root_keeps_its_own_spelling() {
    let root = PathBuf::from("vfs");
    let outside = PathBuf::from("elsewhere").join("keys.toml");
    assert_eq!(store_rel(&root, &outside), outside.display().to_string());
}

#[test]
fn a_path_that_climbs_back_out_of_the_root_keeps_its_own_spelling() {
    let root = PathBuf::from("vfs");
    let climbing = root.join("..").join("elsewhere");
    assert_eq!(store_rel(&root, &climbing), climbing.display().to_string());
}

#[test]
fn a_verify_document_totals_every_entry_it_covered() {
    let doc = VerifyDoc {
        format_version: STORE_FORMAT_VERSION,
        store: "vfs".to_string(),
        subject: "TEST00000".to_string(),
        clean: false,
        entries: vec![
            VerifiedEntryDoc {
                entry: "base".to_string(),
                matched: 3,
                divergences: vec![DivergenceDoc {
                    path: "dev_hdd0/game/TEST00000/PARAM.SFO".to_string(),
                    kind: "missing".to_string(),
                    expected: None,
                    found: None,
                    reason: None,
                }],
            },
            VerifiedEntryDoc {
                entry: "02.51".to_string(),
                matched: 2,
                divergences: Vec::new(),
            },
        ],
    };
    assert_eq!(doc.matched(), 5);
    assert_eq!(doc.diverged(), 1);
}

#[test]
fn a_title_document_serializes_under_the_names_the_schema_declares() {
    let doc = TitleListDoc {
        format_version: STORE_FORMAT_VERSION,
        store: "vfs".to_string(),
        titles: vec![TitleDoc {
            title_id: "TEST00000".to_string(),
            short_name: Some("synthetic".to_string()),
            display_name: None,
            ships_in_firmware: false,
            base: Some(BaseDoc {
                version: "01.00".to_string(),
                dir: "dev_hdd0/game/TEST00000".to_string(),
                tree: "game".to_string(),
                distribution: "psn-hdd".to_string(),
                source_sha256: "ab".repeat(32),
                record: Some(".cellgov/installs/titles/TEST00000/base.install.toml".to_string()),
            }),
            updates: Vec::new(),
            anchors: vec![AnchorDoc {
                fw: "4.91".to_string(),
                game_ver: Some("base".to_string()),
                expect: "frontier".to_string(),
                reference: true,
                recorded: false,
                installed: true,
            }],
        }],
    };
    let json: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&doc).expect("serialize")).expect("parse");

    assert_eq!(json["format_version"], STORE_FORMAT_VERSION);
    let title = &json["titles"][0];
    assert_eq!(title["title_id"], "TEST00000");
    assert_eq!(title["short_name"], "synthetic");
    assert_eq!(title["base"]["version"], "01.00");
    assert!(
        title["base"].get("app_ver").is_none(),
        "the base names its version by the key the record holds it under: {title}"
    );
    assert_eq!(title["anchors"][0]["fw"], "4.91");
    assert_eq!(title["anchors"][0]["recorded"], false);
    assert!(
        title.get("display_name").is_none(),
        "an absent optional field is omitted, not rendered null: {title}"
    );
}
