use super::*;

use std::collections::BTreeMap;

use crate::game_install::sha256_of;
use crate::scratch_dir::{scratch, ScratchDir};
use crate::store::record::{ArtifactRecord, SourceRecord, TitleRecord};
use crate::store::{ArtifactKind, TitleTree, INSTALL_RECORD_FORMAT_VERSION};

/// Placeholder identity: these cases build every tree by hand and name
/// no installed corpus.
const SYNTHETIC_TITLE_ID: &str = "TEST00000";

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("create {}: {e}", parent.display()));
    }
    std::fs::write(path, text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

fn record(kind: ArtifactKind, version: &str, store_path: &str, files: &[&str]) -> InstallRecord {
    let mut recorded = BTreeMap::new();
    for rel in files {
        recorded.insert((*rel).to_string(), sha256_of(rel.as_bytes()));
    }
    InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind,
            version: version.to_string(),
            store_path: store_path.to_string(),
        },
        source: SourceRecord::local("pkg", sha256_of(b"container")),
        title: Some(TitleRecord {
            title_id: SYNTHETIC_TITLE_ID.to_string(),
            content_id: SYNTHETIC_TITLE_ID.to_string(),
            category: "HG".to_string(),
            title: "Synthetic".to_string(),
            distribution: "psn-hdd".to_string(),
        }),
        files: recorded,
        rap: None,
    }
}

/// A store with a base plus `updates`, records and trees both.
fn store_with(updates: &[&str]) -> ScratchDir {
    let root = scratch();
    let layout = StoreLayout::new(&*root);
    let key = TitleId::new(SYNTHETIC_TITLE_ID).expect("synthetic title id");

    let base_path = format!("dev_hdd0/game/{SYNTHETIC_TITLE_ID}");
    let base = record(
        ArtifactKind::TitleBase,
        "01.00",
        &base_path,
        &["USRDIR/EBOOT.BIN"],
    );
    write(&root.join(&base_path).join("USRDIR/EBOOT.BIN"), "eboot");
    write(
        &layout.record_path(&Artifact::TitleBase {
            title_id: key.clone(),
        }),
        &base.to_toml().expect("serialize the base record"),
    );

    for version in updates {
        let artifact = Artifact::TitleUpdate {
            title_id: key.clone(),
            version: VersionKey::new(version).expect("synthetic version"),
        };
        let entry_dir = layout.entry_dir(&artifact);
        let store_path = layout
            .store_path_of(&entry_dir)
            .expect("the entry is under the root");
        let tree = format!("{}/USRDIR/EBOOT.BIN", TitleTree::Game.dir_name());
        write(&entry_dir.join(&tree), "patched eboot");
        write(
            &layout.record_path(&artifact),
            &record(ArtifactKind::TitleUpdate, version, &store_path, &[&tree])
                .to_toml()
                .expect("serialize the update record"),
        );
    }
    root
}

fn versions(plan: &UninstallPlan) -> Vec<String> {
    plan.entries.iter().map(|e| e.version.to_string()).collect()
}

#[test]
fn the_bare_scope_takes_the_base_when_no_update_is_installed() {
    let root = store_with(&[]);
    let plan = plan(SYNTHETIC_TITLE_ID, &root, &UninstallScope::Base).expect("plan the base");
    assert_eq!(versions(&plan), vec!["base"]);
    assert_eq!(plan.recorded_files(), 1);
    assert!(plan.kept_updates.is_empty());
}

#[test]
fn the_bare_scope_is_refused_while_an_update_is_installed() {
    let root = store_with(&["02.10", "02.51"]);
    let err = plan(SYNTHETIC_TITLE_ID, &root, &UninstallScope::Base)
        .expect_err("the base alone would orphan the updates");
    assert!(
        matches!(&err, GameUninstallError::UpdatesInstalled { count, .. } if *count == 2),
        "got {err}"
    );
    let rendered = err.to_string();
    assert!(rendered.contains("02.10, 02.51"), "{rendered}");
    assert!(rendered.contains("--all"), "{rendered}");
    assert!(rendered.contains("--updates"), "{rendered}");
}

#[test]
fn the_all_scope_removes_every_update_before_the_base() {
    let root = store_with(&["02.51", "02.10"]);
    let plan = plan(SYNTHETIC_TITLE_ID, &root, &UninstallScope::All).expect("plan the title");
    assert_eq!(
        versions(&plan),
        vec!["update 02.10", "update 02.51", "base"],
        "updates in version order, base last"
    );
    assert!(plan.kept_updates.is_empty());
}

#[test]
fn the_updates_scope_keeps_the_base() {
    let root = store_with(&["02.10", "02.51"]);
    let plan = plan(SYNTHETIC_TITLE_ID, &root, &UninstallScope::Updates).expect("plan the updates");
    assert_eq!(versions(&plan), vec!["update 02.10", "update 02.51"]);
    assert!(plan.rap.is_none(), "no base, so no RAP");
}

#[test]
fn one_version_leaves_the_others_named_as_kept() {
    let root = store_with(&["02.10", "02.51"]);
    let plan = plan(
        SYNTHETIC_TITLE_ID,
        &root,
        &UninstallScope::Update("02.10".to_string()),
    )
    .expect("plan one update");
    assert_eq!(versions(&plan), vec!["update 02.10"]);
    assert_eq!(plan.kept_updates, vec!["02.51".to_string()]);
}

#[test]
fn a_version_that_is_not_installed_names_the_ones_that_are() {
    let root = store_with(&["02.10"]);
    let err = plan(
        SYNTHETIC_TITLE_ID,
        &root,
        &UninstallScope::Update("09.99".to_string()),
    )
    .expect_err("09.99 is not installed");
    assert!(
        matches!(&err, GameUninstallError::NoUpdateRecord { version, .. } if version == "09.99"),
        "got {err}"
    );
    assert!(err.to_string().contains("02.10"), "{err}");
}

#[test]
fn the_updates_scope_over_a_title_with_none_plans_nothing() {
    let root = store_with(&[]);
    let plan = plan(SYNTHETIC_TITLE_ID, &root, &UninstallScope::Updates).expect("plan the updates");
    assert!(plan.entries.is_empty());
}

#[test]
fn the_updates_scope_over_an_id_that_names_no_entry_is_a_miss() {
    let root = scratch();
    let err = plan(SYNTHETIC_TITLE_ID, &root, &UninstallScope::Updates)
        .expect_err("nothing is installed under this id");
    assert!(
        matches!(&err, GameUninstallError::NoRecord { title_id } if title_id == SYNTHETIC_TITLE_ID),
        "got {err}"
    );
}

/// The `store_path` aims the tombstone rename and the `remove_dir_all`.
#[test]
fn an_update_record_naming_another_versions_tree_is_refused() {
    let root = store_with(&["02.10", "02.51"]);
    let layout = StoreLayout::new(&*root);
    let key = TitleId::new(SYNTHETIC_TITLE_ID).expect("synthetic title id");
    let victim = layout.entry_dir(&Artifact::TitleUpdate {
        title_id: key.clone(),
        version: VersionKey::new("02.51").expect("synthetic version"),
    });
    let named = layout
        .store_path_of(&victim)
        .expect("the entry is under the root");
    write(
        &layout.record_path(&Artifact::TitleUpdate {
            title_id: key,
            version: VersionKey::new("02.10").expect("synthetic version"),
        }),
        &record(ArtifactKind::TitleUpdate, "02.10", &named, &[])
            .to_toml()
            .expect("serialize"),
    );

    let err = plan(
        SYNTHETIC_TITLE_ID,
        &root,
        &UninstallScope::Update("02.10".to_string()),
    )
    .expect_err("02.10's record names 02.51's tree");
    assert!(
        matches!(
            &err,
            GameUninstallError::RecordTreeForeign { store_path, .. } if *store_path == named
        ),
        "got {err}"
    );
    assert!(
        victim.join("game/USRDIR/EBOOT.BIN").exists(),
        "the other version's tree is not this entry's to remove"
    );
}

#[test]
fn a_title_with_no_base_record_is_refused_by_name() {
    let root = scratch();
    let err = plan(SYNTHETIC_TITLE_ID, &root, &UninstallScope::All)
        .expect_err("nothing is installed under this id");
    assert!(
        matches!(&err, GameUninstallError::NoRecord { title_id } if title_id == SYNTHETIC_TITLE_ID),
        "got {err}"
    );
}

#[test]
fn an_update_record_filed_under_a_base_kind_is_refused() {
    let root = store_with(&[]);
    let layout = StoreLayout::new(&*root);
    let key = TitleId::new(SYNTHETIC_TITLE_ID).expect("synthetic title id");
    let artifact = Artifact::TitleUpdate {
        title_id: key,
        version: VersionKey::new("02.10").expect("synthetic version"),
    };
    let store_path = layout
        .store_path_of(&layout.entry_dir(&artifact))
        .expect("the entry is under the root");
    write(
        &layout.record_path(&artifact),
        &record(ArtifactKind::TitleBase, "02.10", &store_path, &[])
            .to_toml()
            .expect("serialize"),
    );

    let err = plan(
        SYNTHETIC_TITLE_ID,
        &root,
        &UninstallScope::Update("02.10".to_string()),
    )
    .expect_err("the record declares the wrong kind");
    assert!(
        matches!(
            &err,
            GameUninstallError::RecordKindMismatch { expected, found, .. }
                if *expected == ArtifactKind::TitleUpdate && *found == ArtifactKind::TitleBase
        ),
        "got {err}"
    );
}
