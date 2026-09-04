//! What the commit sequence leaves behind when it faults part way, and
//! what the target gate answers when it cannot stat the target.

use std::collections::BTreeMap;

use super::*;
use crate::manifest::sha256_of;
use crate::scratch_dir::scratch;

fn digest(byte: u8) -> manifest::Sha256 {
    manifest::Sha256(sha256_of(&[byte]))
}

/// A staged 4.91 whose entry directory already holds an installed tree
/// and the record naming it, as a `--force` replace finds them.
fn staged_over_an_installed_version(root: &Path) -> (PathBuf, Staged) {
    let layout = StoreLayout::new(root);
    let artifact = Artifact::Firmware {
        version: VersionKey::new("4.91").unwrap(),
    };
    let entry_dir = layout.entry_dir(&artifact);
    let record_path = layout.record_path(&artifact);
    let staging_root = layout.firmware_staging_dir();

    std::fs::create_dir_all(staging_root.join(FLASH_MOUNT)).unwrap();
    std::fs::write(
        staging_root.join(FLASH_MOUNT).join(MANIFEST_FILE),
        b"staged",
    )
    .unwrap();

    std::fs::create_dir_all(entry_dir.join(FLASH_MOUNT)).unwrap();
    std::fs::write(entry_dir.join(FLASH_MOUNT).join("stale.sprx"), b"old").unwrap();

    let record = InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind: ArtifactKind::Firmware,
            version: "4.91".to_string(),
            store_path: "firmware/4.91".to_string(),
        },
        source: SourceRecord::local(FIRMWARE_SOURCE_KIND, digest(1)),
        title: None,
        files: BTreeMap::new(),
        rap: None,
    };
    std::fs::create_dir_all(record_path.parent().unwrap()).unwrap();
    std::fs::write(&record_path, record.to_toml().unwrap()).unwrap();

    (
        staging_root,
        Staged {
            record,
            version: "4.91".to_string(),
            record_path,
            entry_dir,
            manifest_entries: 0,
            omissions: Vec::new(),
            files: 1,
            packages: Vec::new(),
            replaced: true,
        },
    )
}

#[test]
fn a_replace_whose_rename_fails_leaves_no_record_over_the_cleared_tree() {
    let dir = scratch();
    let (staging, staged) = staged_over_an_installed_version(&dir);
    // Withdrawing the staged tree is the fault this reproduces: the
    // rename onto the entry directory cannot land.
    std::fs::remove_dir_all(&staging).unwrap();

    assert!(matches!(
        commit(&staging, &staged, &()),
        Err(FirmwareInstallError::CommitFailed { .. })
    ));
    assert!(
        !staged.record_path.exists(),
        "the record outlived the tree it names"
    );
    assert!(
        !staged
            .entry_dir
            .join(FLASH_MOUNT)
            .join("stale.sprx")
            .exists(),
        "the replaced tree was cleared, so the record could not have stayed valid"
    );
    // The residue is what the gate meets on the retry: a fault at the
    // rename leaves the entry cleared, so the retry needs no --force.
    assert!(
        !check_entry(
            &staged.record_path,
            &staged.entry_dir,
            "4.91",
            digest(1),
            false
        )
        .expect("a cleared entry directory reads as free"),
        "a cleared entry is not a replacement"
    );
}

#[test]
fn a_first_install_commits_with_no_record_to_drop() {
    let dir = scratch();
    let layout = StoreLayout::new(&*dir);
    let artifact = Artifact::Firmware {
        version: VersionKey::new("4.93").unwrap(),
    };
    let staging_root = layout.firmware_staging_dir();
    std::fs::create_dir_all(staging_root.join(FLASH_MOUNT)).unwrap();

    let staged = Staged {
        record: InstallRecord {
            format_version: INSTALL_RECORD_FORMAT_VERSION,
            artifact: ArtifactRecord {
                kind: ArtifactKind::Firmware,
                version: "4.93".to_string(),
                store_path: "firmware/4.93".to_string(),
            },
            source: SourceRecord::local(FIRMWARE_SOURCE_KIND, digest(2)),
            title: None,
            files: BTreeMap::new(),
            rap: None,
        },
        version: "4.93".to_string(),
        record_path: layout.record_path(&artifact),
        entry_dir: layout.entry_dir(&artifact),
        manifest_entries: 0,
        omissions: Vec::new(),
        files: 0,
        packages: Vec::new(),
        replaced: false,
    };
    commit(&staging_root, &staged, &()).expect("commit");
    assert!(staged.record_path.is_file());
}

/// A directory where the record belongs: `remove_file` refuses it on
/// every host, and the refusal is not absence.
#[test]
fn a_record_that_cannot_be_removed_is_named_rather_than_passed_over() {
    let dir = scratch();
    let occupied = dir.join("4.91.install.toml");
    std::fs::create_dir_all(&occupied).unwrap();
    assert!(matches!(
        remove_record(&occupied),
        Err(FirmwareInstallError::Io { op: "remove", .. })
    ));
}

#[test]
fn a_target_whose_stat_fails_is_not_reported_free() {
    let dir = scratch();
    assert!(matches!(
        dir_non_empty(&unstattable_path(&dir)),
        Err(FirmwareInstallError::Io { op: "stat", .. })
    ));
}

/// A path whose parent component is a regular file: the stat fails with
/// ENOTDIR rather than reaching the entry.
#[cfg(unix)]
fn unstattable_path(dir: &Path) -> PathBuf {
    let file = dir.join("not-a-directory");
    std::fs::write(&file, b"x").unwrap();
    file.join("4.91")
}

/// Win32 refuses `<` in a name before it looks anything up, so the stat
/// fails with ERROR_INVALID_NAME rather than reporting absence.
#[cfg(windows)]
fn unstattable_path(dir: &Path) -> PathBuf {
    dir.join("4<91")
}
