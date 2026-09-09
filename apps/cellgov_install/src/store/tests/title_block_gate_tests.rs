use super::*;

use crate::game_install::sha256_of;

fn base_record_declaring(system_ver: &str) -> InstallRecord {
    InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind: ArtifactKind::TitleBase,
            version: "01.00".to_string(),
            store_path: "dev_hdd0/game/TEST00000".to_string(),
        },
        source: SourceRecord::local("pkg", sha256_of(b"src")),
        title: Some(TitleRecord {
            title_id: "TEST00000".to_string(),
            content_id: "TEST00000".to_string(),
            category: "HG".to_string(),
            title: "synthetic record".to_string(),
            distribution: "psn-hdd".to_string(),
            system_ver: Some(system_ver.to_string()),
            shipped_firmware: None,
        }),
        files: BTreeMap::new(),
        rap: None,
    }
}

#[test]
fn a_misspelled_title_key_is_refused_rather_than_read_as_absent() {
    let text = base_record_declaring("03.4000")
        .to_toml()
        .expect("serialise");
    assert!(text.contains("\nsystem_ver = \"03.4000\""), "{text}");
    let misspelled = text.replace("\nsystem_ver = ", "\nsystem_version = ");
    let err = InstallRecord::parse(&misspelled).unwrap_err();
    assert!(matches!(err, InstallRecordParseError::Toml(_)), "{err:?}");
}

#[test]
fn a_record_from_the_build_before_the_field_reads_as_declaring_nothing() {
    let text = base_record_declaring("03.4000")
        .to_toml()
        .expect("serialise");
    let older = text.replace("\nsystem_ver = \"03.4000\"", "");
    assert!(!older.contains("system_ver"), "{older}");
    let back = InstallRecord::parse(&older).expect("parse");
    assert_eq!(back.title.and_then(|t| t.system_ver), None);
}
