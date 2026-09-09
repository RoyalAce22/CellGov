//! The commit sequence under the rename policy.

#![cfg_attr(not(feature = "decrypt"), allow(unused_imports, dead_code))]

use super::*;
use crate::scratch_dir::scratch;
use crate::store::{Artifact, ArtifactKind, StoreLayout, TitleId};

/// Placeholder identity: these cases build every tree by hand and name
/// no installed corpus.
const SYNTHETIC_TITLE_ID: &str = "TEST00001";

fn synthetic_rap_name() -> String {
    format!("{SYNTHETIC_TITLE_ID}.rap")
}

fn synthetic_record(rap: Option<RapRecord>) -> InstallRecord {
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
        rap,
    )
}

fn synthetic_staged_rap(layout: &StoreLayout, staging: &Path) -> StagedRap {
    StagedRap {
        staged_path: staging.join("rap").join(synthetic_rap_name()),
        final_path: layout.live_exdata_dir().join(synthetic_rap_name()),
        record: RapRecord {
            filename: synthetic_rap_name(),
            sha256: sha256_of(&[0u8; 16]),
        },
    }
}

#[test]
fn a_rap_rename_that_is_refused_leaves_the_record_and_the_target_untouched() {
    let out = scratch();
    let layout = StoreLayout::new(&*out);
    let artifact = Artifact::TitleBase {
        title_id: TitleId::new(SYNTHETIC_TITLE_ID).expect("synthetic title id"),
    };
    let staging = out
        .join("dev_hdd0")
        .join("game")
        .join(format!(".staging-{SYNTHETIC_TITLE_ID}"));
    let tree = staging.join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("PARAM.SFO"), b"sfo").unwrap();
    let staged_rap = synthetic_staged_rap(&layout, &staging);
    assert!(
        !staged_rap.staged_path.exists(),
        "the fault this reproduces: nothing was staged at the RAP's path"
    );

    let record = synthetic_record(Some(staged_rap.record.clone()));
    let record_path = layout.record_path(&artifact);
    std::fs::create_dir_all(record_path.parent().unwrap()).unwrap();
    std::fs::write(&record_path, b"prior record").unwrap();
    let final_dir = out.join("dev_hdd0").join("game").join(SYNTHETIC_TITLE_ID);
    std::fs::create_dir_all(&final_dir).unwrap();
    std::fs::write(final_dir.join("old"), b"o").unwrap();

    let err = commit(
        &staging,
        &tree,
        &final_dir,
        Some(&staged_rap),
        &record_path,
        &record,
        &(),
    )
    .expect_err("the RAP rename cannot land");
    match err {
        GameInstallError::Rename { path, source } => {
            assert_eq!(
                path, staged_rap.final_path,
                "the refusal names the RAP's target"
            );
            assert_eq!(source.attempts, 1, "an absent source is not outwaited");
            assert_eq!(source.source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected Rename, got {other:?}"),
    }
    assert_eq!(
        std::fs::read(&record_path).expect("the prior record is still there"),
        b"prior record",
        "step two (record removal) never ran"
    );
    assert!(
        final_dir.join("old").is_file(),
        "step three (target clear and tree rename) never ran"
    );
    assert!(
        tree.join("PARAM.SFO").is_file(),
        "the staged tree is still under the staging root"
    );
    assert!(!staged_rap.final_path.exists(), "nothing reached exdata");
}

#[cfg(windows)]
#[test]
fn a_handle_held_under_the_staged_tree_is_outwaited_and_the_sequence_lands_whole() {
    use std::time::Duration;

    use crate::store::rename::RENAME_ATTEMPTS;

    let out = scratch();
    let layout = StoreLayout::new(&*out);
    let artifact = Artifact::TitleBase {
        title_id: TitleId::new(SYNTHETIC_TITLE_ID).expect("synthetic title id"),
    };
    let staging = out
        .join("dev_hdd0")
        .join("game")
        .join(format!(".staging-{SYNTHETIC_TITLE_ID}"));
    let tree = staging.join("tree");
    std::fs::create_dir_all(tree.join("USRDIR")).unwrap();
    std::fs::write(tree.join("USRDIR").join("EBOOT.BIN"), b"e").unwrap();
    let staged_rap = synthetic_staged_rap(&layout, &staging);
    write_and_sync(&staged_rap.staged_path, &[0u8; 16]).expect("stage the rap");
    let record = synthetic_record(Some(staged_rap.record.clone()));
    let record_path = layout.record_path(&artifact);
    let final_dir = out.join("dev_hdd0").join("game").join(SYNTHETIC_TITLE_ID);

    let held = std::fs::File::open(tree.join("USRDIR").join("EBOOT.BIN")).unwrap();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(350));
        drop(held);
    });

    let committed = commit(
        &staging,
        &tree,
        &final_dir,
        Some(&staged_rap),
        &record_path,
        &record,
        &(),
    )
    .expect("lands once the handle closes");
    releaser.join().unwrap();

    assert!(
        committed.rename_retries >= 1,
        "the held tree refused at least the first attempt"
    );
    assert!(
        committed.rename_retries < RENAME_ATTEMPTS,
        "a count of outwaited refusals is below the attempt cap"
    );
    assert_eq!(
        std::fs::read(&staged_rap.final_path).expect("the RAP reached exdata"),
        [0u8; 16]
    );
    assert!(final_dir.join("USRDIR").join("EBOOT.BIN").is_file());
    assert_eq!(committed.record_path, record_path);
    assert!(record_path.is_file(), "the record landed last");
    assert!(!staging.exists(), "the staging root is gone");
}
