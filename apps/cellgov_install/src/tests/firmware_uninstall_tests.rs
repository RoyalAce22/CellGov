use super::*;

use crate::game_install::sha256_of;
use crate::scratch_dir::{scratch, ScratchDir};
use crate::store::record::{ArtifactRecord, SourceRecord};
use crate::store::INSTALL_RECORD_FORMAT_VERSION;

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("create {}: {e}", parent.display()));
    }
    std::fs::write(path, text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

fn firmware_record(version: &str, store_path: &str) -> InstallRecord {
    InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind: ArtifactKind::Firmware,
            version: version.to_string(),
            store_path: store_path.to_string(),
        },
        source: SourceRecord::local("pup", sha256_of(version.as_bytes())),
        title: None,
        files: std::collections::BTreeMap::new(),
        rap: None,
    }
}

/// A store with one entry tree and record per version.
fn store_with(versions: &[&str]) -> ScratchDir {
    let root = scratch();
    let layout = StoreLayout::new(&*root);
    for version in versions {
        let artifact = Artifact::Firmware {
            version: VersionKey::new(version).expect("synthetic version"),
        };
        let entry_dir = layout.entry_dir(&artifact);
        write(&entry_dir.join("dev_flash/vsh/etc/version.txt"), version);
        let store_path = layout
            .store_path_of(&entry_dir)
            .expect("the entry is under the root");
        write(
            &layout.record_path(&artifact),
            &firmware_record(version, &store_path)
                .to_toml()
                .expect("serialize the record"),
        );
    }
    root
}

fn entry_dir(root: &Path, version: &str) -> PathBuf {
    StoreLayout::new(root).entry_dir(&Artifact::Firmware {
        version: VersionKey::new(version).expect("synthetic version"),
    })
}

#[test]
fn installed_versions_reads_the_record_filenames_in_order() {
    let root = store_with(&["4.91", "3.55"]);
    assert_eq!(
        installed_versions(&root).expect("enumerate the records"),
        vec!["3.55".to_string(), "4.91".to_string()]
    );
}

/// The listing is what a `NoRecord` refusal offers as the alternatives.
#[test]
fn a_record_filename_that_is_no_version_key_is_not_listed_as_installed() {
    let root = store_with(&["4.91"]);
    let records = StoreLayout::new(&*root)
        .installs_dir()
        .join(ArtifactKind::Firmware.as_str());
    write(&records.join(INSTALL_RECORD_SUFFIX), "");
    write(&records.join(format!(".{INSTALL_RECORD_SUFFIX}")), "");

    assert_eq!(
        installed_versions(&root).expect("enumerate the records"),
        vec!["4.91".to_string()]
    );
}

#[test]
fn a_root_with_no_records_lists_nothing() {
    let root = scratch();
    assert!(installed_versions(&root)
        .expect("an absent directory is an empty list")
        .is_empty());
}

#[test]
fn the_plan_names_the_entry_and_the_record_without_touching_them() {
    let root = store_with(&["4.91"]);
    let plan = plan("4.91", &root).expect("plan the removal");
    assert_eq!(plan.entry_dir, entry_dir(&root, "4.91"));
    assert_eq!(plan.pup_sha256, sha256_of(b"4.91").to_hex());
    assert!(plan.entry_dir.exists(), "planning removes nothing");
    assert!(plan.record_path.exists());
}

#[test]
fn uninstall_removes_the_entry_the_record_and_the_tombstone() {
    let root = store_with(&["4.91", "3.55"]);
    let outcome = uninstall("4.91", &root).expect("uninstall");
    assert_eq!(outcome.version, "4.91");
    assert!(!entry_dir(&root, "4.91").exists());
    assert!(!outcome.record_removed.exists());
    assert!(!tombstone_sibling(&entry_dir(&root, "4.91"))
        .expect("an entry dir names an entry")
        .exists());
    assert!(
        entry_dir(&root, "3.55").exists(),
        "the other installed version is untouched"
    );
    assert_eq!(
        installed_versions(&root).expect("enumerate"),
        vec!["3.55".to_string()]
    );
}

#[test]
fn a_version_that_is_not_installed_names_the_ones_that_are() {
    let root = store_with(&["4.91"]);
    let err = plan("9.99", &root).expect_err("9.99 is not installed");
    assert!(
        matches!(&err, FirmwareUninstallError::NoRecord { version, .. } if version == "9.99"),
        "got {err}"
    );
    assert!(err.to_string().contains("4.91"), "{err}");
}

#[test]
fn a_stale_tombstone_is_swept_before_the_rename() {
    let root = store_with(&["4.91"]);
    let stale = tombstone_sibling(&entry_dir(&root, "4.91")).expect("an entry dir names an entry");
    write(&stale.join("residue"), "from an interrupted run");

    uninstall("4.91", &root).expect("uninstall over the stale tombstone");
    assert!(!stale.exists());
    assert!(!entry_dir(&root, "4.91").exists());
}

#[test]
fn an_entry_whose_tree_is_already_gone_still_removes_the_record() {
    let root = store_with(&["4.91"]);
    std::fs::remove_dir_all(entry_dir(&root, "4.91")).expect("remove the tree by hand");

    let outcome = uninstall("4.91", &root).expect("the record still names what to clean up");
    assert!(!outcome.record_removed.exists());
    assert!(installed_versions(&root).expect("enumerate").is_empty());
}

#[test]
fn a_record_declaring_another_kind_is_refused() {
    let root = store_with(&[]);
    let layout = StoreLayout::new(&*root);
    let artifact = Artifact::Firmware {
        version: VersionKey::new("4.91").expect("synthetic version"),
    };
    let store_path = layout
        .store_path_of(&layout.entry_dir(&artifact))
        .expect("the entry is under the root");
    let mut record = firmware_record("4.91", &store_path);
    record.artifact.kind = ArtifactKind::TitleBase;
    record.title = Some(crate::store::TitleRecord {
        title_id: "TEST00000".to_string(),
        content_id: "TEST00000".to_string(),
        category: "HG".to_string(),
        title: "Synthetic".to_string(),
        distribution: "psn-hdd".to_string(),
    });
    write(
        &layout.record_path(&artifact),
        &record.to_toml().expect("serialize"),
    );

    let err = plan("4.91", &root).expect_err("the record declares the wrong kind");
    assert!(
        matches!(
            &err,
            FirmwareUninstallError::RecordKindMismatch { found, .. }
                if *found == ArtifactKind::TitleBase
        ),
        "got {err}"
    );
}

/// The `store_path` aims the `remove_dir_all`.
#[test]
fn a_record_naming_another_versions_tree_is_refused() {
    let root = store_with(&["4.91", "3.55"]);
    let layout = StoreLayout::new(&*root);
    let artifact = Artifact::Firmware {
        version: VersionKey::new("4.91").expect("synthetic version"),
    };
    let other = layout
        .store_path_of(&entry_dir(&root, "3.55"))
        .expect("the entry is under the root");
    write(
        &layout.record_path(&artifact),
        &firmware_record("4.91", &other)
            .to_toml()
            .expect("serialize"),
    );

    let err = plan("4.91", &root).expect_err("the record names 3.55's tree");
    assert!(
        matches!(err, FirmwareUninstallError::RecordTreeForeign { .. }),
        "got {err}"
    );
    assert!(entry_dir(&root, "3.55").exists());
}

/// A version key accepts letters and never folds case, so the miscased
/// version names the same record only on a case-folding volume. The
/// test probes this volume first, then expects the refusal that
/// follows.
#[test]
fn a_version_differing_only_in_case_names_no_entry() {
    let root = store_with(&["4.91B"]);
    let miscased = "4.91b";
    let folds_case = StoreLayout::new(&*root)
        .record_path(&Artifact::Firmware {
            version: VersionKey::new(miscased).expect("synthetic version"),
        })
        .exists();

    let err = plan(miscased, &root).expect_err("only 4.91B is installed");
    if folds_case {
        assert!(
            matches!(&err, FirmwareUninstallError::RecordTreeForeign { .. }),
            "the record is readable here, so the refusal is the identity check; got {err}"
        );
    } else {
        assert!(
            matches!(&err, FirmwareUninstallError::NoRecord { .. }),
            "got {err}"
        );
    }
    assert!(
        entry_dir(&root, "4.91B")
            .join("dev_flash/vsh/etc/version.txt")
            .exists(),
        "the installed version's tree is untouched"
    );
}

#[test]
fn a_version_that_is_not_a_store_directory_name_is_refused() {
    let root = store_with(&[]);
    for bad in ["../4.91", "4.91/x", ".hidden", ""] {
        let err = plan(bad, &root).expect_err("must be refused");
        assert!(
            matches!(err, FirmwareUninstallError::UnsafeVersion { .. }),
            "{bad:?} got {err}"
        );
    }
}
