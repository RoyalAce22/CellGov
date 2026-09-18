//! `installed_record`: a record, and nothing else in the entry
//! directory, makes a firmware version installed.

use std::collections::BTreeMap;

use super::*;
use crate::manifest::sha256_of;
use crate::scratch_dir::scratch;
use crate::store::layout::TitleId;
use crate::store::record::TitleRecord;

fn write(path: &Path, record: &InstallRecord) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, record.to_toml().unwrap()).unwrap();
}

fn firmware_record(version: &str) -> InstallRecord {
    InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind: ArtifactKind::Firmware,
            version: version.to_string(),
            store_path: format!("firmware/{version}"),
        },
        source: SourceRecord::local(FIRMWARE_SOURCE_KIND, manifest::Sha256(sha256_of(b"pup"))),
        title: None,
        files: BTreeMap::new(),
        rap: None,
        core_os: None,
    }
}

#[test]
fn a_recorded_version_is_installed_and_the_record_comes_back() {
    let root = scratch();
    let layout = StoreLayout::new(&*root);
    let artifact = Artifact::Firmware {
        version: VersionKey::new("2.76").unwrap(),
    };
    write(&layout.record_path(&artifact), &firmware_record("2.76"));
    let found = installed_record(&root, "2.76")
        .expect("a readable record")
        .expect("the version is installed");
    assert_eq!(found.artifact.version, "2.76");
    assert_eq!(found.source.sha256, manifest::Sha256(sha256_of(b"pup")));
}

#[test]
fn an_unrecorded_tree_is_not_an_installed_version() {
    let root = scratch();
    let layout = StoreLayout::new(&*root);
    let entry = layout.entry_dir(&Artifact::Firmware {
        version: VersionKey::new("2.76").unwrap(),
    });
    std::fs::create_dir_all(entry.join(FLASH_MOUNT)).unwrap();
    std::fs::write(entry.join(FLASH_MOUNT).join("residue"), b"x").unwrap();
    assert!(installed_record(&root, "2.76").unwrap().is_none());
}

#[test]
fn a_version_that_is_not_a_store_key_is_refused_before_any_read() {
    let root = scratch();
    assert!(matches!(
        installed_record(&root, "../2.76"),
        Err(FirmwareInstallError::StoreKey(_))
    ));
}

#[test]
fn a_record_this_build_will_not_read_is_refused_not_absent() {
    let root = scratch();
    let layout = StoreLayout::new(&*root);
    let path = layout.record_path(&Artifact::Firmware {
        version: VersionKey::new("2.76").unwrap(),
    });
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "format_version = 99\n").unwrap();
    assert!(matches!(
        installed_record(&root, "2.76"),
        Err(FirmwareInstallError::RecordParse { .. })
    ));
}

#[test]
fn a_record_that_is_there_and_unreadable_is_refused_not_absent() {
    let root = scratch();
    let layout = StoreLayout::new(&*root);
    let path = layout.record_path(&Artifact::Firmware {
        version: VersionKey::new("2.76").unwrap(),
    });
    // A directory where the record file should be: every host refuses
    // to read it as a file, and none reports it as not found.
    std::fs::create_dir_all(&path).unwrap();
    assert!(matches!(
        installed_record(&root, "2.76"),
        Err(FirmwareInstallError::Io { op: "read", .. })
    ));
}

#[test]
fn a_record_under_the_key_that_describes_something_else_is_refused() {
    let root = scratch();
    let layout = StoreLayout::new(&*root);
    let artifact = Artifact::Firmware {
        version: VersionKey::new("2.76").unwrap(),
    };
    let mut other = firmware_record("2.76");
    other.artifact.kind = ArtifactKind::TitleBase;
    other.title = Some(TitleRecord {
        title_id: TitleId::new("TEST00000").unwrap().as_str().to_string(),
        content_id: "TEST00000".to_string(),
        category: "DG".to_string(),
        title: "T".to_string(),
        distribution: "disc-iso".to_string(),
        system_ver: None,
        shipped_firmware: None,
    });
    write(&layout.record_path(&artifact), &other);
    assert!(matches!(
        installed_record(&root, "2.76"),
        Err(FirmwareInstallError::RecordMismatch { .. })
    ));

    write(&layout.record_path(&artifact), &firmware_record("2.80"));
    assert!(matches!(
        installed_record(&root, "2.76"),
        Err(FirmwareInstallError::RecordMismatch { .. })
    ));
}
