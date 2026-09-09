//! What the commit sequence leaves behind when it faults part way, and
//! what the target gate answers when it cannot stat the target.

#![cfg_attr(not(feature = "decrypt"), allow(unused_imports, dead_code))]

use super::*;
use crate::scratch_dir::scratch;
use crate::store::{Artifact, ArtifactKind, StoreLayout, TitleId};

/// Placeholder identity: these cases build every tree by hand and name
/// no installed corpus.
const SYNTHETIC_TITLE_ID: &str = "TEST00001";

fn synthetic_record() -> InstallRecord {
    build_record(
        "pkg",
        b"src-bytes",
        ArtifactRecord {
            kind: ArtifactKind::TitleBase,
            version: "01.00".to_string(),
            store_path: format!("dev_hdd0/game/{SYNTHETIC_TITLE_ID}"),
        },
        BTreeMap::new(),
        TitleRecord {
            title_id: SYNTHETIC_TITLE_ID.to_string(),
            content_id: SYNTHETIC_TITLE_ID.to_string(),
            category: "HG".to_string(),
            title: "T".to_string(),
            distribution: "psn-hdd".to_string(),
        },
        None,
    )
}

#[test]
fn a_replace_whose_rename_fails_leaves_no_record_over_the_cleared_tree() {
    let out = scratch();
    let layout = StoreLayout::new(&*out);
    let artifact = Artifact::TitleBase {
        title_id: TitleId::new(SYNTHETIC_TITLE_ID).expect("synthetic title id"),
    };
    let record = synthetic_record();
    let record_path = layout.record_path(&artifact);
    std::fs::create_dir_all(record_path.parent().unwrap()).unwrap();
    std::fs::write(&record_path, record.to_toml().unwrap()).unwrap();

    let final_dir = out.join("dev_hdd0").join("game").join(SYNTHETIC_TITLE_ID);
    std::fs::create_dir_all(&final_dir).unwrap();
    std::fs::write(final_dir.join("old"), b"o").unwrap();

    // Naming a staging root that was never built is the fault this
    // reproduces: the rename onto the target cannot land.
    let staging = out
        .join("dev_hdd0")
        .join("game")
        .join(format!(".staging-{SYNTHETIC_TITLE_ID}"));

    assert!(matches!(
        commit(
            &staging,
            &staging,
            &final_dir,
            None,
            &record_path,
            &record,
            &(),
        ),
        Err(GameInstallError::Rename { .. })
    ));
    assert!(
        !record_path.exists(),
        "the record outlived the tree it names"
    );
    assert!(
        !final_dir.join("old").exists(),
        "the replaced tree was cleared, so the record could not have stayed valid"
    );
    assert!(
        !dir_non_empty(&final_dir).expect("stat the cleared target"),
        "a cleared target refuses nothing, so this retry needs no --force"
    );
}

/// The other residue shape: the tree landed and the record write did
/// not, so the target holds a tree no record names.
#[test]
fn a_commit_over_an_unrecorded_tree_clears_it_and_lands_the_record() {
    let out = scratch();
    let layout = StoreLayout::new(&*out);
    let artifact = Artifact::TitleBase {
        title_id: TitleId::new(SYNTHETIC_TITLE_ID).expect("synthetic title id"),
    };
    let record_path = layout.record_path(&artifact);
    let final_dir = out.join("dev_hdd0").join("game").join(SYNTHETIC_TITLE_ID);
    std::fs::create_dir_all(&final_dir).unwrap();
    std::fs::write(final_dir.join("residue.bin"), b"x").unwrap();

    assert!(
        dir_non_empty(&final_dir).expect("stat the residue"),
        "the target gate has to see the unrecorded tree"
    );

    let staging = out
        .join("dev_hdd0")
        .join("game")
        .join(format!(".staging-{SYNTHETIC_TITLE_ID}"));
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("PARAM.SFO"), b"sfo").unwrap();
    commit(
        &staging,
        &staging,
        &final_dir,
        None,
        &record_path,
        &synthetic_record(),
        &(),
    )
    .expect("commit");

    assert!(!final_dir.join("residue.bin").exists());
    assert!(final_dir.join("PARAM.SFO").is_file());
    assert!(record_path.is_file());
}

#[test]
fn a_first_install_commits_with_no_record_to_drop() {
    let out = scratch();
    let layout = StoreLayout::new(&*out);
    let artifact = Artifact::TitleBase {
        title_id: TitleId::new(SYNTHETIC_TITLE_ID).expect("synthetic title id"),
    };
    let record_path = layout.record_path(&artifact);
    let staging = out
        .join("dev_hdd0")
        .join("game")
        .join(format!(".staging-{SYNTHETIC_TITLE_ID}"));
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("PARAM.SFO"), b"sfo").unwrap();

    commit(
        &staging,
        &staging,
        &out.join("dev_hdd0").join("game").join(SYNTHETIC_TITLE_ID),
        None,
        &record_path,
        &synthetic_record(),
        &(),
    )
    .expect("commit");
    assert!(record_path.is_file());
}

/// A directory where the record belongs: `remove_file` refuses it on
/// every host, and the refusal is not absence.
#[test]
fn a_record_that_cannot_be_removed_is_named_rather_than_passed_over() {
    let out = scratch();
    let occupied = out.join(format!("{SYNTHETIC_TITLE_ID}.install.toml"));
    std::fs::create_dir_all(&occupied).unwrap();
    assert!(matches!(
        remove_record(&occupied),
        Err(GameInstallError::Io { op: "remove", .. })
    ));
}

#[test]
fn a_target_whose_stat_fails_is_not_reported_free() {
    let out = scratch();
    assert!(matches!(
        dir_non_empty(&unstattable_path(&out)),
        Err(GameInstallError::Io { op: "stat", .. })
    ));
}

/// A path whose parent component is a regular file: the stat fails with
/// ENOTDIR rather than reaching the entry.
#[cfg(unix)]
fn unstattable_path(dir: &Path) -> PathBuf {
    let file = dir.join("not-a-directory");
    std::fs::write(&file, b"x").unwrap();
    file.join(SYNTHETIC_TITLE_ID)
}

/// Win32 refuses `<` in a name before it looks anything up, so the stat
/// fails with ERROR_INVALID_NAME rather than reporting absence.
#[cfg(windows)]
fn unstattable_path(dir: &Path) -> PathBuf {
    dir.join("TEST<0001")
}
