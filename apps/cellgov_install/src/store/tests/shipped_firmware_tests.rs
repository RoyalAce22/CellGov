//! The `[title] shipped_firmware` key: round trip, absence, and the
//! path-component gate.

use super::*;

use crate::game_install::sha256_of;

fn disc_record(shipped_firmware: Option<&str>) -> InstallRecord {
    InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind: ArtifactKind::TitleBase,
            version: "02.00".to_string(),
            store_path: "dev_bdvd/TEST00000".to_string(),
        },
        source: SourceRecord::local("iso", sha256_of(b"iso")),
        title: Some(TitleRecord {
            title_id: "TEST00000".to_string(),
            content_id: "TEST00000".to_string(),
            category: "DG".to_string(),
            title: "synthetic disc".to_string(),
            distribution: "disc-iso".to_string(),
            system_ver: Some("02.7600".to_string()),
            shipped_firmware: shipped_firmware.map(str::to_string),
        }),
        files: BTreeMap::new(),
        rap: None,
    }
}

#[test]
fn a_shipped_firmware_round_trips_under_title_and_is_omitted_when_absent() {
    let text = disc_record(None).to_toml().expect("serialise");
    assert!(!text.contains("shipped_firmware"), "{text}");
    let back = InstallRecord::parse(&text).expect("parse");
    assert_eq!(back.title.and_then(|t| t.shipped_firmware), None);

    let text = disc_record(Some("2.76")).to_toml().expect("serialise");
    let title_block = text
        .split("[title]")
        .nth(1)
        .expect("the record carries a [title] block")
        .split("\n[")
        .next()
        .expect("a block")
        .to_string();
    assert!(
        title_block.contains("\nshipped_firmware = \"2.76\""),
        "{text}"
    );
    let back = InstallRecord::parse(&text).expect("parse");
    assert_eq!(
        back.title.and_then(|t| t.shipped_firmware).as_deref(),
        Some("2.76")
    );
}

#[test]
fn a_record_from_the_build_before_the_field_reads_as_shipping_nothing() {
    let text = disc_record(Some("2.76")).to_toml().expect("serialise");
    let older = text.replace("\nshipped_firmware = \"2.76\"", "");
    assert!(!older.contains("shipped_firmware"), "{older}");
    let back = InstallRecord::parse(&older).expect("parse");
    assert_eq!(back.title.and_then(|t| t.shipped_firmware), None);
}

#[test]
fn a_misspelled_shipped_firmware_key_is_refused_rather_than_read_as_absent() {
    let text = disc_record(Some("2.76")).to_toml().expect("serialise");
    let misspelled = text.replace("\nshipped_firmware = ", "\nshipped_fw = ");
    let err = InstallRecord::parse(&misspelled).unwrap_err();
    assert!(matches!(err, InstallRecordParseError::Toml(_)), "{err:?}");
}

#[test]
fn a_shipped_firmware_that_is_not_one_path_component_is_refused() {
    for bad in [
        "", "../4.91", "4.91/x", "4.91\\x", "4 91", ".staging", "NUL", "com1", "4.91.",
    ] {
        let err = InstallRecord::parse(&disc_record(Some(bad)).to_toml().expect("serialise"))
            .unwrap_err();
        assert!(
            matches!(
                err,
                InstallRecordParseError::UnsafeTitleKey {
                    field: "shipped_firmware",
                    ..
                }
            ),
            "{bad:?} was accepted: {err:?}"
        );
    }
}

#[test]
fn an_update_record_carrying_a_shipped_firmware_is_refused() {
    let mut record = disc_record(Some("2.76"));
    record.artifact.kind = ArtifactKind::TitleUpdate;
    record.artifact.version = "01.02".to_string();
    record.artifact.store_path = "dev_hdd0/game/TEST00000/updates/01.02".to_string();
    let err = InstallRecord::parse(&record.to_toml().expect("serialise")).unwrap_err();
    assert!(
        matches!(
            err,
            InstallRecordParseError::UnexpectedShippedFirmware { ref version } if version == "2.76"
        ),
        "{err:?}"
    );

    let mut record = disc_record(None);
    record.artifact.kind = ArtifactKind::TitleUpdate;
    record.artifact.version = "01.02".to_string();
    record.artifact.store_path = "dev_hdd0/game/TEST00000/updates/01.02".to_string();
    InstallRecord::parse(&record.to_toml().expect("serialise"))
        .expect("an update record without the key parses");
}
