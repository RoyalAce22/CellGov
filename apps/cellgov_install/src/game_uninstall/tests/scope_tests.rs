//! What each [`UninstallScope`] removes, and what it leaves: the
//! base-plus-updates cases the single-entry teardown never sees.

use super::*;

use std::collections::BTreeMap;

use crate::game_install::sha256_of;
use crate::scratch_dir::{scratch, ScratchDir};
use crate::store::layout::{Artifact, StoreLayout, TitleId, VersionKey};
use crate::store::record::{
    ArtifactRecord, InstallRecord, RapRecord, SourceRecord, TitleRecord,
    INSTALL_RECORD_FORMAT_VERSION,
};
use crate::store::{ArtifactKind, TitleTree};

/// Placeholder identity: every tree here is hand-written and names no
/// installed content.
const TITLE_ID: &str = "TEST00000";

const NO_VERIFY: UninstallOptions = UninstallOptions {
    verify: false,
    keep_rap: false,
    force: false,
};

const VERIFY: UninstallOptions = UninstallOptions {
    verify: true,
    keep_rap: false,
    force: false,
};

fn write(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("create {}: {e}", parent.display()));
    }
    std::fs::write(path, bytes).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

fn base_record_path(vfs: &Path) -> PathBuf {
    StoreLayout::new(vfs).record_path(&Artifact::TitleBase {
        title_id: TitleId::new(TITLE_ID).expect("synthetic title id"),
    })
}

fn update_artifact(version: &str) -> Artifact {
    Artifact::TitleUpdate {
        title_id: TitleId::new(TITLE_ID).expect("synthetic title id"),
        version: VersionKey::new(version).expect("synthetic version"),
    }
}

fn update_tree(vfs: &Path, version: &str) -> PathBuf {
    StoreLayout::new(vfs).entry_dir(&update_artifact(version))
}

fn base_tree(vfs: &Path) -> PathBuf {
    vfs.join("dev_hdd0").join("game").join(TITLE_ID)
}

fn rap_path(vfs: &Path) -> PathBuf {
    StoreLayout::new(vfs)
        .live_exdata_dir()
        .join("UP0000-TEST00000_00-X.rap")
}

/// A base tree plus its RAP and record.
fn stage_base(vfs: &Path) {
    let tree = base_tree(vfs);
    write(&tree.join("USRDIR/EBOOT.BIN"), b"eboot");
    let rap = rap_path(vfs);
    write(&rap, &[7u8; 16]);

    let layout = StoreLayout::new(vfs);
    let mut files = BTreeMap::new();
    files.insert("USRDIR/EBOOT.BIN".to_string(), sha256_of(b"eboot"));
    let record = InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind: ArtifactKind::TitleBase,
            version: "01.00".to_string(),
            store_path: layout.store_path_of(&tree).expect("under the vfs root"),
        },
        source: SourceRecord::local("pkg", sha256_of(b"base-src")),
        title: Some(TitleRecord {
            title_id: TITLE_ID.to_string(),
            content_id: TITLE_ID.to_string(),
            category: "HG".to_string(),
            title: "Synthetic".to_string(),
            distribution: "psn-hdd".to_string(),
            system_ver: None,
            shipped_firmware: None,
        }),
        files,
        rap: Some(RapRecord {
            filename: "UP0000-TEST00000_00-X.rap".to_string(),
            sha256: sha256_of(&[7u8; 16]),
        }),
        core_os: None,
    };
    write(
        &base_record_path(vfs),
        record
            .to_toml()
            .expect("serialize the base record")
            .as_bytes(),
    );
}

/// One update version's tree and record over an installed base.
fn stage_update(vfs: &Path, version: &str) {
    let layout = StoreLayout::new(vfs);
    let artifact = update_artifact(version);
    let entry_dir = layout.entry_dir(&artifact);
    let rel = format!("{}/USRDIR/EBOOT.BIN", TitleTree::Game.dir_name());
    let bytes = format!("patched by {version}").into_bytes();
    write(&entry_dir.join(&rel), &bytes);

    let mut files = BTreeMap::new();
    files.insert(rel, sha256_of(&bytes));
    let record = InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind: ArtifactKind::TitleUpdate,
            version: version.to_string(),
            store_path: layout
                .store_path_of(&entry_dir)
                .expect("under the vfs root"),
        },
        source: SourceRecord::local("pkg", sha256_of(&bytes)),
        title: Some(TitleRecord {
            title_id: TITLE_ID.to_string(),
            content_id: TITLE_ID.to_string(),
            category: "GD".to_string(),
            title: "Synthetic".to_string(),
            distribution: "psn-update".to_string(),
            system_ver: None,
            shipped_firmware: None,
        }),
        files,
        rap: None,
        core_os: None,
    };
    write(
        &layout.record_path(&artifact),
        record
            .to_toml()
            .expect("serialize the update record")
            .as_bytes(),
    );
}

/// A base and two updates, trees and records both.
fn store_with_two_updates() -> ScratchDir {
    let out = scratch();
    let vfs = out.join("vfs");
    stage_base(&vfs);
    stage_update(&vfs, "02.10");
    stage_update(&vfs, "02.51");
    out
}

#[test]
fn the_all_scope_leaves_neither_a_tree_nor_a_record_behind() {
    let out = store_with_two_updates();
    let vfs = out.join("vfs");

    let outcome =
        uninstall(TITLE_ID, &vfs, &UninstallScope::All, NO_VERIFY).expect("uninstall the title");
    assert_eq!(outcome.removed.len(), 3);
    assert!(!base_tree(&vfs).exists());
    assert!(!update_tree(&vfs, "02.10").exists());
    assert!(!update_tree(&vfs, "02.51").exists());
    assert!(!base_record_path(&vfs).exists());
    assert_eq!(
        outcome.rap_removed.as_deref(),
        Some(rap_path(&vfs).as_path())
    );
    assert!(outcome.kept_updates.is_empty());
}

#[test]
fn the_updates_scope_leaves_the_base_tree_its_record_and_its_rap() {
    let out = store_with_two_updates();
    let vfs = out.join("vfs");

    let outcome = uninstall(TITLE_ID, &vfs, &UninstallScope::Updates, NO_VERIFY)
        .expect("uninstall the updates");
    assert_eq!(outcome.removed.len(), 2);
    assert!(outcome.base().is_none());
    assert!(base_tree(&vfs).join("USRDIR/EBOOT.BIN").exists());
    assert!(base_record_path(&vfs).exists());
    assert!(rap_path(&vfs).exists(), "the RAP goes with the base");
    assert!(outcome.rap_removed.is_none());
    assert!(!update_tree(&vfs, "02.10").exists());
    assert!(!update_tree(&vfs, "02.51").exists());
}

#[test]
fn one_version_leaves_the_other_update_installed() {
    let out = store_with_two_updates();
    let vfs = out.join("vfs");

    let outcome = uninstall(
        TITLE_ID,
        &vfs,
        &UninstallScope::Update("02.10".to_string()),
        NO_VERIFY,
    )
    .expect("uninstall one update");
    assert_eq!(outcome.kept_updates, vec!["02.51".to_string()]);
    assert!(!update_tree(&vfs, "02.10").exists());
    assert!(update_tree(&vfs, "02.51").exists());
    assert!(base_tree(&vfs).join("USRDIR/EBOOT.BIN").exists());
}

#[test]
fn the_bare_scope_removes_nothing_while_updates_are_installed() {
    let out = store_with_two_updates();
    let vfs = out.join("vfs");

    let err = uninstall(TITLE_ID, &vfs, &UninstallScope::Base, NO_VERIFY)
        .expect_err("the base alone would orphan both updates");
    assert!(
        matches!(err, GameUninstallError::UpdatesInstalled { .. }),
        "got {err:?}"
    );
    assert!(base_tree(&vfs).join("USRDIR/EBOOT.BIN").exists());
    assert!(update_tree(&vfs, "02.10").exists());
}

/// The gate runs over the whole plan before the first rename.
#[test]
fn a_divergence_in_the_last_entry_stops_the_whole_removal() {
    let out = store_with_two_updates();
    let vfs = out.join("vfs");
    write(&base_tree(&vfs).join("USRDIR/EBOOT.BIN"), b"tampered");

    let err = uninstall(TITLE_ID, &vfs, &UninstallScope::All, VERIFY)
        .expect_err("the base tree was modified");
    assert!(
        matches!(err, GameUninstallError::TreeModified { .. }),
        "got {err:?}"
    );
    assert!(
        update_tree(&vfs, "02.10").exists(),
        "the updates are removed before the base, so a base divergence must stop \
         the run before the first rename"
    );
}

#[test]
fn verify_over_the_whole_title_counts_every_entry() {
    let out = store_with_two_updates();
    let vfs = out.join("vfs");

    let outcome =
        uninstall(TITLE_ID, &vfs, &UninstallScope::All, VERIFY).expect("every tree is intact");
    // One recorded file per update, one in the base, plus the RAP.
    assert_eq!(outcome.files_verified, Some(4));
    assert_eq!(outcome.files_diverged, Some(0));
}

#[test]
fn an_already_removed_update_is_idempotent() {
    let out = store_with_two_updates();
    let vfs = out.join("vfs");
    let scope = UninstallScope::Update("02.10".to_string());

    uninstall(TITLE_ID, &vfs, &scope, NO_VERIFY).expect("first uninstall");
    let err = uninstall(TITLE_ID, &vfs, &scope, NO_VERIFY)
        .expect_err("the version is no longer installed");
    assert!(
        matches!(err, GameUninstallError::NoUpdateRecord { .. }),
        "got {err:?}"
    );
}
