use super::*;

use crate::game_install::sha256_of;

/// Placeholder identity: these name no real title and no installed
/// corpus.
const SYNTHETIC_TITLE_ID: &str = "TEST00000";
const SYNTHETIC_CONTENT_ID: &str = "TT0000-TEST00000_00-SYNTHETICRECORD0";

fn title_block() -> TitleRecord {
    TitleRecord {
        title_id: SYNTHETIC_TITLE_ID.to_string(),
        content_id: SYNTHETIC_CONTENT_ID.to_string(),
        category: "HG".to_string(),
        title: "synthetic record".to_string(),
        distribution: "psn-hdd".to_string(),
    }
}

fn base_record() -> InstallRecord {
    InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind: ArtifactKind::TitleBase,
            version: "01.00".to_string(),
            store_path: format!("dev_hdd0/game/{SYNTHETIC_TITLE_ID}"),
        },
        source: SourceRecord::local("pkg", sha256_of(b"src")),
        title: Some(title_block()),
        files: [("USRDIR/EBOOT.BIN".to_string(), sha256_of(b"eboot"))]
            .into_iter()
            .collect(),
        rap: None,
    }
}

fn firmware_record() -> InstallRecord {
    InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind: ArtifactKind::Firmware,
            version: "4.91".to_string(),
            store_path: "firmware/4.91".to_string(),
        },
        source: SourceRecord::local("pup", sha256_of(b"pup")),
        title: None,
        files: BTreeMap::new(),
        rap: None,
    }
}

fn toml_of(record: &InstallRecord) -> String {
    record.to_toml().expect("serialise")
}

#[test]
fn a_base_record_round_trips() {
    let back = InstallRecord::parse(&toml_of(&base_record())).expect("parse");
    assert_eq!(back.format_version, INSTALL_RECORD_FORMAT_VERSION);
    assert_eq!(back.artifact.kind, ArtifactKind::TitleBase);
    assert_eq!(back.artifact.version, "01.00");
    assert_eq!(
        back.artifact.store_path,
        format!("dev_hdd0/game/{SYNTHETIC_TITLE_ID}")
    );
    assert_eq!(
        back.title.map(|t| t.title_id).as_deref(),
        Some(SYNTHETIC_TITLE_ID)
    );
    assert!(back.files.contains_key("USRDIR/EBOOT.BIN"));
}

#[test]
fn a_firmware_record_round_trips_without_a_title_or_files() {
    let text = toml_of(&firmware_record());
    assert!(!text.contains("[title]"), "{text}");
    assert!(!text.contains("[files]"), "{text}");
    let back = InstallRecord::parse(&text).expect("parse");
    assert_eq!(back.artifact.kind, ArtifactKind::Firmware);
    assert!(back.title.is_none());
    assert!(back.files.is_empty());
}

#[test]
fn a_rap_present_record_round_trips() {
    let mut record = base_record();
    record.rap = Some(RapRecord {
        filename: format!("{SYNTHETIC_CONTENT_ID}.rap"),
        sha256: sha256_of(b"rap bytes"),
    });
    let text = toml_of(&record);
    assert!(text.contains("[rap]"), "{text}");
    assert!(text.contains("[files]"), "{text}");
    assert!(!text.contains("[[files]]"), "{text}");
    let back = InstallRecord::parse(&text).expect("parse");
    assert_eq!(
        back.rap.map(|r| r.filename),
        Some(format!("{SYNTHETIC_CONTENT_ID}.rap"))
    );
}

#[test]
fn a_rap_absent_record_emits_no_rap_table() {
    assert!(!toml_of(&base_record()).contains("[rap]"));
}

#[test]
fn acquisition_fields_are_absent_for_a_hand_supplied_container() {
    let text = toml_of(&base_record());
    for field in [
        "url",
        "fetched_at",
        "sha1_head",
        "ver_xml_cached",
        "min_system_ver",
    ] {
        assert!(!text.contains(field), "{field} emitted for a local source");
    }
    let back = InstallRecord::parse(&text).expect("parse");
    assert!(back.source.url.is_none());
    assert!(back.source.fetched_at.is_none());
}

#[test]
fn acquisition_fields_round_trip_when_present() {
    let mut record = base_record();
    record.source.url = Some("https://example.invalid/p.pkg".to_string());
    record.source.fetched_at = Some("2026-08-28T00:00:00Z".to_string());
    record.source.sha1_head = Some("da39a3ee5e6b4b0d3255bfef95601890afd80709".to_string());
    record.source.ver_xml_cached = Some(".cellgov/fetch/titles/TEST00000/ver.xml".to_string());
    record.source.min_system_ver = Some("03.55".to_string());
    let back = InstallRecord::parse(&toml_of(&record)).expect("parse");
    assert_eq!(
        back.source.url.as_deref(),
        Some("https://example.invalid/p.pkg")
    );
    assert_eq!(back.source.min_system_ver.as_deref(), Some("03.55"));
}

#[test]
fn a_record_from_another_schema_is_refused_by_version() {
    let text = toml_of(&base_record());
    for found in [0, 1, 2, INSTALL_RECORD_FORMAT_VERSION + 1] {
        let other = text.replace(
            &format!("format_version = {INSTALL_RECORD_FORMAT_VERSION}"),
            &format!("format_version = {found}"),
        );
        let err = InstallRecord::parse(&other).unwrap_err();
        assert!(
            matches!(
                err,
                InstallRecordParseError::UnsupportedFormatVersion { found: f, supported }
                    if f == found && supported == INSTALL_RECORD_FORMAT_VERSION
            ),
            "{err:?}"
        );
    }
}

/// A record from the schema before this one carries no `[artifact]`
/// block.
#[test]
fn a_record_from_the_schema_before_this_one_is_refused_by_its_version() {
    let text = "format_version = 2\n\
                [source]\nkind = \"pkg\"\nsha256 = \"00000000000000000000000000000000000000000000000000000000000000ff\"\n\
                [title]\ntitle_id = \"TEST00000\"\ncontent_id = \"TEST00000\"\ncategory = \"HG\"\n\
                title = \"T\"\napp_version = \"01.00\"\ndistribution = \"psn-hdd\"\n\
                [files]\n";
    let err = InstallRecord::parse(text).unwrap_err();
    assert!(
        matches!(
            err,
            InstallRecordParseError::UnsupportedFormatVersion { found: 2, supported }
                if supported == INSTALL_RECORD_FORMAT_VERSION
        ),
        "{err:?}"
    );
    let msg = err.to_string();
    assert!(msg.contains("cellgov title install"), "{msg}");
}

#[test]
fn a_record_that_is_not_toml_is_refused() {
    let err = InstallRecord::parse("this is not toml {{{").unwrap_err();
    assert!(matches!(err, InstallRecordParseError::Toml(_)), "{err:?}");
}

#[test]
fn a_title_record_without_a_title_block_is_refused() {
    let text = toml_of(&base_record());
    let stripped: String = text
        .lines()
        .take_while(|l| !l.starts_with("[title]"))
        .collect::<Vec<_>>()
        .join("\n");
    let err = InstallRecord::parse(&stripped).unwrap_err();
    assert!(
        matches!(
            err,
            InstallRecordParseError::MissingTitleBlock {
                kind: ArtifactKind::TitleBase
            }
        ),
        "{err:?}"
    );
}

#[test]
fn a_firmware_record_with_a_title_block_is_refused() {
    let mut record = firmware_record();
    record.title = Some(title_block());
    let err = InstallRecord::parse(&toml_of(&record)).unwrap_err();
    assert!(
        matches!(err, InstallRecordParseError::UnexpectedTitleBlock),
        "{err:?}"
    );
}

#[test]
fn a_firmware_record_with_a_rap_block_is_refused() {
    let mut record = firmware_record();
    record.rap = Some(RapRecord {
        filename: "x.rap".to_string(),
        sha256: sha256_of(b"rap"),
    });
    let err = InstallRecord::parse(&toml_of(&record)).unwrap_err();
    assert!(
        matches!(err, InstallRecordParseError::UnexpectedRapBlock),
        "{err:?}"
    );
}

#[test]
fn a_store_path_that_could_leave_the_vfs_root_is_refused() {
    for bad in [
        "",
        "/absolute",
        "../escape",
        "dev_hdd0/../../escape",
        "dev_hdd0//game",
        "dev_hdd0\\game",
        ".cellgov/keys",
    ] {
        let mut record = base_record();
        record.artifact.store_path = bad.to_string();
        let err = InstallRecord::parse(&toml_of(&record)).unwrap_err();
        assert!(
            matches!(err, InstallRecordParseError::UnsafeStorePath { .. }),
            "{bad:?} was accepted: {err:?}"
        );
    }
}

#[test]
fn a_version_the_store_path_encodes_must_be_a_directory_name() {
    for kind in [ArtifactKind::Firmware, ArtifactKind::TitleUpdate] {
        for bad in ["", "../4.91", "4.91/x", ".staging"] {
            let mut record = if kind == ArtifactKind::Firmware {
                firmware_record()
            } else {
                let mut r = base_record();
                r.artifact.kind = ArtifactKind::TitleUpdate;
                r
            };
            record.artifact.version = bad.to_string();
            let err = InstallRecord::parse(&toml_of(&record)).unwrap_err();
            assert!(
                matches!(
                    err,
                    InstallRecordParseError::UnsafeArtifactVersion { kind: k, .. } if k == kind
                ),
                "{kind:?} accepted version {bad:?}: {err:?}"
            );
        }
    }
}

/// A base's version is its `APP_VER`, which the store path does not
/// encode and which a container may not carry at all.
#[test]
fn a_base_version_is_free_text() {
    for version in ["", "01.00", "1.0-rev2"] {
        let mut record = base_record();
        record.artifact.version = version.to_string();
        let back = InstallRecord::parse(&toml_of(&record)).expect("parse");
        assert_eq!(back.artifact.version, version);
    }
}

#[test]
fn a_misspelled_block_is_refused_rather_than_read_as_absent() {
    let mut record = base_record();
    record.rap = Some(RapRecord {
        filename: format!("{SYNTHETIC_CONTENT_ID}.rap"),
        sha256: sha256_of(b"rap bytes"),
    });
    let text = toml_of(&record).replace("[rap]", "[raps]");
    let err = InstallRecord::parse(&text).unwrap_err();
    assert!(matches!(err, InstallRecordParseError::Toml(_)), "{err:?}");
}

/// Uninstall reads every `[files]` key under the recorded tree.
#[test]
fn a_files_key_that_could_leave_the_recorded_tree_is_refused() {
    for bad in [
        "/etc/passwd",
        "../escape",
        "USRDIR/../../escape",
        "USRDIR//EBOOT.BIN",
        "USRDIR/./EBOOT.BIN",
        "USRDIR\\EBOOT.BIN",
        "C:/windows/system32",
        "EBOOT.BIN:stream",
    ] {
        let mut record = base_record();
        record.files = [(bad.to_string(), sha256_of(b"x"))].into_iter().collect();
        let err = InstallRecord::parse(&toml_of(&record)).unwrap_err();
        assert!(
            matches!(err, InstallRecordParseError::UnsafeFilePath { .. }),
            "{bad:?} was accepted: {err:?}"
        );
    }
    // An empty key names the tree root itself; it is written here rather
    // than round-tripped so the check does not depend on how the
    // serializer renders one.
    let text = toml_of(&base_record());
    assert!(text.contains("\"USRDIR/EBOOT.BIN\""), "{text}");
    let empty_key = text.replace("\"USRDIR/EBOOT.BIN\"", "\"\"");
    let err = InstallRecord::parse(&empty_key).unwrap_err();
    assert!(
        matches!(err, InstallRecordParseError::UnsafeFilePath { .. }),
        "an empty [files] key was accepted: {err:?}"
    );
}

#[test]
fn a_files_key_may_carry_the_characters_a_container_entry_name_does() {
    for ok in ["USRDIR/EBOOT.BIN", "USRDIR/My Data.bin", "PARAM.SFO"] {
        let mut record = base_record();
        record.files = [(ok.to_string(), sha256_of(b"x"))].into_iter().collect();
        let back = InstallRecord::parse(&toml_of(&record)).expect("parse");
        assert!(back.files.contains_key(ok), "{ok:?} was refused");
    }
}

/// Uninstall joins the RAP filename onto the live exdata directory and
/// removes the result.
#[test]
fn a_rap_filename_that_is_not_one_path_component_is_refused() {
    for bad in ["", "../../evil", "a/b.rap", ".hidden.rap", "x.rap."] {
        let mut record = base_record();
        record.rap = Some(RapRecord {
            filename: bad.to_string(),
            sha256: sha256_of(b"rap"),
        });
        let err = InstallRecord::parse(&toml_of(&record)).unwrap_err();
        assert!(
            matches!(err, InstallRecordParseError::UnsafeRapFilename { .. }),
            "{bad:?} was accepted: {err:?}"
        );
    }
}

/// The `[title]` ids spell a store directory and the filename the
/// manifest generator writes.
#[test]
fn a_title_id_or_content_id_that_is_not_one_path_component_is_refused() {
    for (field, bad) in [
        ("title_id", ".."),
        ("title_id", "../../evil"),
        ("title_id", ""),
        ("content_id", "a/b"),
        ("content_id", ".hidden"),
    ] {
        let mut record = base_record();
        let block = record.title.as_mut().expect("base record has a [title]");
        if field == "title_id" {
            block.title_id = bad.to_string();
        } else {
            block.content_id = bad.to_string();
        }
        let err = InstallRecord::parse(&toml_of(&record)).unwrap_err();
        assert!(
            matches!(
                err,
                InstallRecordParseError::UnsafeTitleKey { field: f, .. } if f == field
            ),
            "{field} accepted {bad:?}: {err:?}"
        );
    }
}

#[test]
fn a_block_shape_refusal_outranks_the_key_checks() {
    let mut record = firmware_record();
    record.title = Some(TitleRecord {
        title_id: "../escape".to_string(),
        ..title_block()
    });
    let err = InstallRecord::parse(&toml_of(&record)).unwrap_err();
    assert!(
        matches!(err, InstallRecordParseError::UnexpectedTitleBlock),
        "{err:?}"
    );
}

#[test]
fn artifact_kinds_serialise_under_the_names_a_reader_sees() {
    for (kind, name) in [
        (ArtifactKind::Firmware, "firmware"),
        (ArtifactKind::TitleBase, "title-base"),
        (ArtifactKind::TitleUpdate, "title-update"),
    ] {
        assert_eq!(kind.as_str(), name);
        let mut record = base_record();
        record.artifact.kind = kind;
        assert!(
            toml_of(&record).contains(&format!("kind = \"{name}\"")),
            "{kind:?} did not serialise as {name}"
        );
    }
}

#[test]
fn a_firmware_record_with_a_files_table_is_refused() {
    let mut record = firmware_record();
    record.files = [("vsh/etc/version.txt".to_string(), sha256_of(b"4.91"))]
        .into_iter()
        .collect();
    let err = InstallRecord::parse(&toml_of(&record)).unwrap_err();
    assert!(
        matches!(err, InstallRecordParseError::UnexpectedFilesBlock),
        "{err:?}"
    );
}
