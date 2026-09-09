//! The game half's version: the tree that boots names it, under one
//! PARAM.SFO key, and the record must agree.

use std::path::Path;

use cellgov_compare::{AppVersion, RunIdentity};

use super::{GameIdentityError, IdentityError};
use crate::composition::compose::{compose_boot, ComposeError, ComposeInputs};
use crate::composition::test_support::SyntheticStore;
use crate::game::manifest::{CheckpointTrigger, Distribution, GameSource, TitleManifest};

const DISABLE_ENV: &str = "CELLGOV_NO_FIRMWARE_DIR";

fn manifest(content_id: &str, source: GameSource) -> TitleManifest {
    TitleManifest {
        content_id: content_id.to_string(),
        short_name: "t".to_string(),
        display_name: "Synthetic".to_string(),
        eboot_candidates: vec!["EBOOT.BIN".to_string()],
        year: 2007,
        developer: "test-developer".to_string(),
        engine: "test-engine".to_string(),
        distribution: Distribution::PsnHdd,
        rap_filename: None,
        bench_max_steps: None,
        system_ver: None,
        checkpoint: CheckpointTrigger::ProcessExit,
        source,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
        matrix: Vec::new(),
    }
}

fn compose(
    title: &TitleManifest,
    store: &SyntheticStore,
    vfs_root: &Path,
    game_ver: Option<&str>,
) -> Result<RunIdentity, ComposeError> {
    compose_boot(&ComposeInputs {
        title,
        vfs_root,
        install_root: store.root(),
        fw: None,
        game_ver,
        firmware_dir: None,
        no_firmware: false,
        disable_env: DISABLE_ENV,
    })
    .map(|c| c.identity)
}

fn game_of(id: RunIdentity) -> Option<AppVersion> {
    id.game.expect("a store entry was composed").app_version
}

fn game_refusal(err: ComposeError) -> GameIdentityError {
    match err {
        ComposeError::Identity(IdentityError::Game(e)) => e,
        other => panic!("expected a title identity refusal, got {other}"),
    }
}

#[test]
fn a_base_whose_table_carries_only_version_is_named_by_that_key() {
    let store = SyntheticStore::new("gid_version_only");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.02", false);
    store.write_base_param_sfo(
        "NPAA00001",
        false,
        &[("TITLE_ID", "NPAA00001"), ("VERSION", "01.02")],
    );
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let id = compose(&title, &store, &vfs, None).expect("composes");
    assert_eq!(
        game_of(id),
        Some(AppVersion::SfoVersion("01.02".to_string())),
        "the identity names the key the tree used, not APP_VER"
    );
}

#[test]
fn an_empty_app_ver_yields_to_version_as_the_installer_did() {
    let store = SyntheticStore::new("gid_empty_app_ver");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.02", false);
    store.write_base_param_sfo(
        "NPAA00001",
        false,
        &[
            ("TITLE_ID", "NPAA00001"),
            ("APP_VER", ""),
            ("VERSION", "01.02"),
        ],
    );
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let id = compose(&title, &store, &vfs, None).expect("composes");
    assert_eq!(
        game_of(id),
        Some(AppVersion::SfoVersion("01.02".to_string()))
    );
}

#[test]
fn a_base_naming_no_version_reads_as_none_when_the_record_agrees() {
    let store = SyntheticStore::new("gid_no_version");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "", false);
    store.write_base_param_sfo("NPAA00001", false, &[("TITLE_ID", "NPAA00001")]);
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let id = compose(&title, &store, &vfs, None).expect("composes");
    assert_eq!(game_of(id), None);
}

#[test]
fn a_disc_base_reads_the_table_under_ps3_game() {
    let store = SyntheticStore::new("gid_disc");
    store.add_firmware("4.93", true);
    store.add_base("BLAA00001", "02.00", true);
    store.write_base_param_sfo(
        "BLAA00001",
        true,
        &[("TITLE_ID", "BLAA00001"), ("VERSION", "02.00")],
    );
    let title = manifest("BLAA00001", GameSource::Disc);
    let vfs = store.root().join("dev_hdd0");
    let id = compose(&title, &store, &vfs, None).expect("composes");
    assert_eq!(
        game_of(id),
        Some(AppVersion::SfoVersion("02.00".to_string()))
    );
}

#[test]
fn a_table_disagreeing_with_the_record_is_refused_naming_both() {
    let store = SyntheticStore::new("gid_mismatch");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    store.write_base_param_sfo(
        "NPAA00001",
        false,
        &[("TITLE_ID", "NPAA00001"), ("APP_VER", "01.05")],
    );
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let err = game_refusal(
        compose(&title, &store, &vfs, None)
            .expect_err("the tree and the record name different versions"),
    );
    let GameIdentityError::Mismatch {
        path,
        recorded,
        found,
    } = &err
    else {
        panic!("expected a mismatch, got {err:?}");
    };
    assert_eq!(*path, store.base_param_sfo("NPAA00001", false));
    assert_eq!(recorded, "01.00");
    assert_eq!(*found, Some(AppVersion::AppVer("01.05".to_string())));
    let msg = err.to_string();
    assert!(
        msg.contains("app_ver 01.05") && msg.contains("\"01.00\""),
        "the refusal names both claims: {msg}"
    );
}

#[test]
fn a_table_naming_no_version_against_a_record_that_does_is_refused() {
    let store = SyntheticStore::new("gid_mismatch_none");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    store.write_base_param_sfo("NPAA00001", false, &[("TITLE_ID", "NPAA00001")]);
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let err = game_refusal(
        compose(&title, &store, &vfs, None)
            .expect_err("the tree names no version and the record does"),
    );
    assert!(
        matches!(
            &err,
            GameIdentityError::Mismatch {
                recorded,
                found: None,
                ..
            } if recorded == "01.00"
        ),
        "{err:?}"
    );
    let msg = err.to_string();
    assert!(msg.contains("no version key"), "{msg}");
}

#[test]
fn a_tree_with_no_table_is_refused_naming_the_file() {
    let store = SyntheticStore::new("gid_missing");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    std::fs::remove_file(store.base_param_sfo("NPAA00001", false)).unwrap();
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let err = game_refusal(
        compose(&title, &store, &vfs, None)
            .expect_err("a tree with no PARAM.SFO cannot name its version"),
    );
    assert!(
        matches!(
            &err,
            GameIdentityError::Read { path, .. } if *path == store.base_param_sfo("NPAA00001", false)
        ),
        "{err:?}"
    );
}

#[test]
fn a_table_that_does_not_parse_is_refused_naming_the_file() {
    let store = SyntheticStore::new("gid_corrupt");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    std::fs::write(store.base_param_sfo("NPAA00001", false), b"not an sfo").unwrap();
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let err = game_refusal(
        compose(&title, &store, &vfs, None).expect_err("a corrupt PARAM.SFO names nothing"),
    );
    assert!(
        matches!(
            &err,
            GameIdentityError::Parse { path, .. } if *path == store.base_param_sfo("NPAA00001", false)
        ),
        "{err:?}"
    );
}

#[test]
fn a_selected_update_is_named_by_its_own_table() {
    let store = SyntheticStore::new("gid_update_key");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    std::fs::write(
        store.update_tree("NPAA00001", "02.51").join("PARAM.SFO"),
        cellgov_testkit::param_sfo::build_param_sfo(&[
            ("TITLE_ID", "NPAA00001"),
            ("VERSION", "02.51"),
        ]),
    )
    .unwrap();
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let id = compose(&title, &store, &vfs, Some("02.51")).expect("composes");
    assert_eq!(
        game_of(id),
        Some(AppVersion::SfoVersion("02.51".to_string())),
        "the base's APP_VER does not stand in for the update's table"
    );
}

#[test]
fn an_update_over_a_disc_base_is_named_by_the_update_table_not_the_disc() {
    let store = SyntheticStore::new("gid_disc_update");
    store.add_firmware("4.93", true);
    store.add_base("BLAA00001", "02.00", true);
    store.add_update("BLAA00001", "02.51");
    let title = manifest("BLAA00001", GameSource::Disc);
    let vfs = store.root().join("dev_hdd0");
    let id = compose(&title, &store, &vfs, Some("02.51")).expect("composes");
    let game = id.game.expect("a store entry was composed");
    assert_eq!(game.version, "update:02.51");
    assert_eq!(
        game.app_version,
        Some(AppVersion::AppVer("02.51".to_string())),
        "the disc's PS3_GAME/PARAM.SFO does not name a composed update"
    );
}

#[test]
fn an_update_table_disagreeing_with_its_record_is_refused_naming_the_update_file() {
    let store = SyntheticStore::new("gid_update_mismatch");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    std::fs::write(
        store.update_tree("NPAA00001", "02.51").join("PARAM.SFO"),
        cellgov_testkit::param_sfo::build_param_sfo(&[
            ("TITLE_ID", "NPAA00001"),
            ("APP_VER", "02.60"),
        ]),
    )
    .unwrap();
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let err = game_refusal(
        compose(&title, &store, &vfs, Some("02.51"))
            .expect_err("the update tree and its record name different versions"),
    );
    let GameIdentityError::Mismatch {
        path,
        recorded,
        found,
    } = &err
    else {
        panic!("expected a mismatch, got {err:?}");
    };
    assert_eq!(
        *path,
        store.update_tree("NPAA00001", "02.51").join("PARAM.SFO"),
        "the refusal names the update's own table, not the base's"
    );
    assert_eq!(recorded, "02.51");
    assert_eq!(*found, Some(AppVersion::AppVer("02.60".to_string())));
}

#[test]
fn the_two_keys_fingerprint_apart_for_one_version_string() {
    let store = SyntheticStore::new("gid_fingerprint");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let by_app_ver = compose(&title, &store, &vfs, None).expect("composes");
    store.write_base_param_sfo(
        "NPAA00001",
        false,
        &[("TITLE_ID", "NPAA00001"), ("VERSION", "01.00")],
    );
    let by_version = compose(&title, &store, &vfs, None).expect("composes");
    assert_ne!(by_app_ver, by_version);
    assert_ne!(by_app_ver.game_fingerprint(), by_version.game_fingerprint());
}
