//! A tombstone rename refused partway through a multi-entry removal:
//! what is gone, what stands, and what the refusal names.

use super::*;

use std::cell::Cell;
use std::collections::BTreeMap;
use std::io;

use crate::game_install::sha256_of;
use crate::scratch_dir::{scratch, ScratchDir};
use crate::store::layout::{Artifact, StoreLayout, TitleId, VersionKey};
use crate::store::record::{
    ArtifactRecord, InstallRecord, SourceRecord, TitleRecord, INSTALL_RECORD_FORMAT_VERSION,
};
use crate::store::rename::RENAME_ATTEMPTS;
use crate::store::{ArtifactKind, TitleTree};

/// Placeholder identity: every tree here is hand-written and names no
/// installed corpus.
const TITLE_ID: &str = "TEST00000";

/// The one update over the base; the plan removes it first.
const UPDATE: &str = "02.10";

const NO_VERIFY: UninstallOptions = UninstallOptions {
    verify: false,
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

fn title_id() -> TitleId {
    TitleId::new(TITLE_ID).expect("synthetic title id")
}

fn base_artifact() -> Artifact {
    Artifact::TitleBase {
        title_id: title_id(),
    }
}

fn update_artifact() -> Artifact {
    Artifact::TitleUpdate {
        title_id: title_id(),
        version: VersionKey::new(UPDATE).expect("synthetic version"),
    }
}

fn base_tree(vfs: &Path) -> PathBuf {
    vfs.join("dev_hdd0").join("game").join(TITLE_ID)
}

fn update_tree(vfs: &Path) -> PathBuf {
    StoreLayout::new(vfs).entry_dir(&update_artifact())
}

/// A record over one file, `rel`, holding `bytes`.
fn record(
    kind: ArtifactKind,
    version: &str,
    store_path: String,
    rel: &str,
    bytes: &[u8],
) -> InstallRecord {
    let mut files = BTreeMap::new();
    files.insert(rel.to_string(), sha256_of(bytes));
    InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind,
            version: version.to_string(),
            store_path,
        },
        source: SourceRecord::local("pkg", sha256_of(b"src")),
        title: Some(TitleRecord {
            title_id: TITLE_ID.to_string(),
            content_id: TITLE_ID.to_string(),
            category: "HG".to_string(),
            title: "Synthetic".to_string(),
            distribution: "psn-hdd".to_string(),
            system_ver: None,
        }),
        files,
        rap: None,
    }
}

/// A base and one update, trees and records both, no RAP.
fn store_with_one_update() -> ScratchDir {
    let out = scratch();
    let vfs = out.join("vfs");
    let layout = StoreLayout::new(&vfs);

    let base = base_tree(&vfs);
    write(&base.join("USRDIR/EBOOT.BIN"), b"eboot");
    let base_record = record(
        ArtifactKind::TitleBase,
        "01.00",
        layout.store_path_of(&base).expect("under the vfs root"),
        "USRDIR/EBOOT.BIN",
        b"eboot",
    );
    write(
        &layout.record_path(&base_artifact()),
        base_record.to_toml().expect("serialize").as_bytes(),
    );

    let update = update_tree(&vfs);
    let rel = format!("{}/USRDIR/EBOOT.BIN", TitleTree::Game.dir_name());
    write(&update.join(&rel), b"patched");
    let update_record = record(
        ArtifactKind::TitleUpdate,
        UPDATE,
        layout.store_path_of(&update).expect("under the vfs root"),
        &rel,
        b"patched",
    );
    write(
        &layout.record_path(&update_artifact()),
        update_record.to_toml().expect("serialize").as_bytes(),
    );
    out
}

/// The refusal the policy reports once every attempt is spent.
fn spent() -> RenameRefused {
    RenameRefused {
        attempts: RENAME_ATTEMPTS,
        source: io::Error::from(io::ErrorKind::PermissionDenied),
    }
}

/// The real rename, reporting `outwaited` refusals when it lands.
fn landing(from: &Path, to: &Path, outwaited: u32) -> Result<u32, RenameRefused> {
    std::fs::rename(from, to)
        .map(|()| outwaited)
        .map_err(|source| RenameRefused {
            attempts: 1,
            source,
        })
}

#[test]
fn the_plan_removes_the_update_before_the_base() {
    let out = store_with_one_update();
    let plan = plan(TITLE_ID, &out.join("vfs"), &UninstallScope::All).expect("plan");
    assert_eq!(plan.entries.len(), 2);
    assert_eq!(
        plan.entries[0].version,
        EntryVersion::Update(UPDATE.to_string())
    );
    assert_eq!(plan.entries[1].version, EntryVersion::Base);
}

#[test]
fn a_rename_refused_on_the_second_entry_leaves_the_first_gone_and_the_second_whole() {
    let out = store_with_one_update();
    let vfs = out.join("vfs");
    let layout = StoreLayout::new(&vfs);
    let plan = plan(TITLE_ID, &vfs, &UninstallScope::All).expect("plan");

    let calls = Cell::new(0u32);
    let err = execute_with(&plan, NO_VERIFY, |from, to| {
        calls.set(calls.get() + 1);
        if calls.get() == 2 {
            Err(spent())
        } else {
            landing(from, to, 0)
        }
    })
    .expect_err("the base's rename stays refused");

    let GameUninstallError::Rename { path, source } = &err else {
        panic!("expected Rename, got {err:?}");
    };
    assert_eq!(
        *path,
        base_tree(&vfs),
        "the refusal names the second entry's tree"
    );
    assert_eq!(source.attempts, RENAME_ATTEMPTS);
    assert!(
        err.to_string()
            .contains(&format!("still refused after {RENAME_ATTEMPTS} attempts")),
        "{err}"
    );

    // The first entry went through its whole teardown.
    assert!(!update_tree(&vfs).exists(), "the update tree is gone");
    assert!(
        !layout.record_path(&update_artifact()).exists(),
        "the update record is gone"
    );
    assert!(
        !tombstone_sibling(&update_tree(&vfs))
            .expect("tombstone")
            .exists(),
        "the update's tombstone is gone"
    );
    // The second entry was not touched past its tombstone sweep.
    assert!(
        base_tree(&vfs).join("USRDIR/EBOOT.BIN").is_file(),
        "the base tree stands"
    );
    assert!(
        layout.record_path(&base_artifact()).is_file(),
        "the base record stands"
    );
    assert_eq!(calls.get(), 2, "no rename was attempted past the refusal");
}

#[test]
fn a_refusal_on_the_first_entry_removes_nothing() {
    let out = store_with_one_update();
    let vfs = out.join("vfs");
    let layout = StoreLayout::new(&vfs);
    let plan = plan(TITLE_ID, &vfs, &UninstallScope::All).expect("plan");

    let err = execute_with(&plan, NO_VERIFY, |_, _| Err(spent())).expect_err("refused at once");
    assert!(
        matches!(&err, GameUninstallError::Rename { path, .. } if *path == update_tree(&vfs)),
        "{err:?}"
    );
    assert!(update_tree(&vfs).exists());
    assert!(layout.record_path(&update_artifact()).is_file());
    assert!(base_tree(&vfs).exists());
    assert!(layout.record_path(&base_artifact()).is_file());
}

#[test]
fn refusals_each_entry_outwaited_are_summed_over_the_removal() {
    let out = store_with_one_update();
    let vfs = out.join("vfs");
    let plan = plan(TITLE_ID, &vfs, &UninstallScope::All).expect("plan");

    let calls = Cell::new(0u32);
    let outcome = execute_with(&plan, NO_VERIFY, |from, to| {
        calls.set(calls.get() + 1);
        landing(from, to, if calls.get() == 1 { 3 } else { 2 })
    })
    .expect("both renames land");
    assert_eq!(outcome.rename_retries, 5);
    assert_eq!(outcome.removed.len(), 2);
    assert!(!base_tree(&vfs).exists());
    assert!(!update_tree(&vfs).exists());
}

#[test]
fn an_absent_tree_is_not_offered_to_the_rename() {
    let out = store_with_one_update();
    let vfs = out.join("vfs");
    std::fs::remove_dir_all(update_tree(&vfs)).expect("remove the update tree by hand");
    let plan = plan(TITLE_ID, &vfs, &UninstallScope::All).expect("plan");

    let calls = Cell::new(0u32);
    let outcome = execute_with(&plan, NO_VERIFY, |from, to| {
        calls.set(calls.get() + 1);
        landing(from, to, 0)
    })
    .expect("the absent tree is idempotent");
    assert_eq!(calls.get(), 1, "only the base tree was renamed");
    assert_eq!(outcome.removed.len(), 2);
}
