//! The identity triple a composed boot names itself by.

use std::path::Path;

use cellgov_compare::AppVersion;

use super::super::composition::{compose_boot, BootComposition, ComposeInputs};
use crate::manifest::{CheckpointTrigger, Distribution, GameSource, TitleManifest};
use cellgov_testkit::store::{firmware_pup_sha256, image_version, SyntheticStore};

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

fn compose<'a>(
    title: &'a TitleManifest,
    store: &'a SyntheticStore,
    vfs_root: &'a Path,
    fw: Option<&'a str>,
    game_ver: Option<&'a str>,
) -> BootComposition {
    compose_boot(&ComposeInputs {
        title,
        vfs_root,
        install_root: store.root(),
        fw,
        game_ver,
        firmware_dir: None,
        no_firmware: false,
    })
    .expect("composes")
}

#[test]
fn a_base_boot_names_both_halves() {
    let store = SyntheticStore::new("id_base");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let id = compose(&title, &store, &vfs, None, None).identity;

    let fw = id.firmware.expect("a managed firmware entry was selected");
    assert_eq!(fw.version, "4.93");
    assert_eq!(fw.image_version, image_version("4.93"));
    assert_eq!(fw.pup_sha256.len(), 64);

    let game = id.game.expect("a store entry was composed");
    assert_eq!(game.title_id, "NPAA00001");
    assert_eq!(game.version, "base");
    assert_eq!(
        game.app_version,
        Some(AppVersion::AppVer("01.00".to_string()))
    );
}

#[test]
fn a_selected_update_names_its_own_version_and_app_ver() {
    let store = SyntheticStore::new("id_update");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let id = compose(&title, &store, &vfs, None, Some("02.51")).identity;

    let game = id.game.expect("a store entry was composed");
    assert_eq!(game.version, "update:02.51");
    assert_eq!(
        game.app_version,
        Some(AppVersion::AppVer("02.51".to_string())),
        "the executable comes from the update tree, so its APP_VER is the update's"
    );
}

#[test]
fn two_firmware_entries_produce_different_triples() {
    let store = SyntheticStore::new("id_two_fw");
    store.add_firmware("4.91", true);
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let a = compose(&title, &store, &vfs, Some("4.91"), None).identity;
    let b = compose(&title, &store, &vfs, Some("4.93"), None).identity;
    assert_ne!(a, b);
    assert_ne!(a.firmware_fingerprint(), b.firmware_fingerprint());
    assert_eq!(a.game_fingerprint(), b.game_fingerprint());
}

#[test]
fn a_title_with_no_store_entry_names_no_game_half() {
    let store = SyntheticStore::new("id_unstored");
    store.add_firmware("4.93", true);
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    std::fs::create_dir_all(vfs.join("game").join("NPAA00001").join("USRDIR")).unwrap();
    let id = compose(&title, &store, &vfs, None, None).identity;
    assert!(id.game.is_none());
    assert!(id.firmware.is_some());
}

#[test]
fn an_unmanaged_firmware_tree_names_no_firmware_half() {
    let store = SyntheticStore::new("id_unmanaged");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let dir = store.firmware_dev_flash("4.93");
    let c = compose_boot(&ComposeInputs {
        title: &title,
        vfs_root: &vfs,
        install_root: store.root(),
        fw: None,
        game_ver: None,
        firmware_dir: Some(&dir),
        no_firmware: false,
    })
    .expect("composes");
    assert!(
        c.identity.firmware.is_none(),
        "a tree outside the store carries no version key to name"
    );
    assert!(c.identity.game.is_some());
}

#[test]
fn a_firmware_entry_with_no_manifest_is_refused() {
    let store = SyntheticStore::new("id_no_manifest");
    store.add_firmware("4.93", true);
    std::fs::remove_file(store.firmware_dev_flash("4.93").join("firmware.toml")).unwrap();
    store.add_base("NPAA00001", "01.00", false);
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let err = compose_boot(&ComposeInputs {
        title: &title,
        vfs_root: &vfs,
        install_root: store.root(),
        fw: None,
        game_ver: None,
        firmware_dir: None,
        no_firmware: false,
    })
    .expect_err("a firmware entry that cannot name its PUP is not composable");
    assert!(
        err.to_string().contains("firmware.toml"),
        "the refusal names the file: {err}"
    );
}

/// Overwrite one firmware entry's manifest with a `[firmware]` block
/// naming `version` and `pup_sha256`, and no `[[files]]` entries.
fn write_manifest(store: &SyntheticStore, entry: &str, version: &str, pup_sha256: &str) {
    std::fs::write(
        store.firmware_dev_flash(entry).join("firmware.toml"),
        format!(
            "format_version = {}\n\n\
             [firmware]\n\
             image_version = \"0x0000000000000000\"\n\
             version = \"{version}\"\n\
             pup_sha256 = \"{pup_sha256}\"\n",
            cellgov_install::manifest::SUPPORTED_FORMAT_VERSION,
        ),
    )
    .unwrap();
}

fn compose_err(store: &SyntheticStore, title: &TitleManifest, vfs_root: &Path) -> String {
    compose_boot(&ComposeInputs {
        title,
        vfs_root,
        install_root: store.root(),
        fw: None,
        game_ver: None,
        firmware_dir: None,
        no_firmware: false,
    })
    .expect_err("the entry cannot name its PUP")
    .to_string()
}

#[test]
fn an_unparsable_manifest_is_refused() {
    let store = SyntheticStore::new("id_bad_manifest");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    std::fs::write(
        store.firmware_dev_flash("4.93").join("firmware.toml"),
        "format_version = 1\n[firmware]\n",
    )
    .unwrap();
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let msg = compose_err(&store, &title, &vfs);
    assert!(msg.contains("firmware.toml"), "{msg}");
}

#[test]
fn a_manifest_naming_another_version_is_refused() {
    let store = SyntheticStore::new("id_manifest_version");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    write_manifest(&store, "4.93", "4.91", &firmware_pup_sha256());
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let msg = compose_err(&store, &title, &vfs);
    assert!(
        msg.contains("4.91") && msg.contains("4.93"),
        "the refusal names both claims: {msg}"
    );
}

#[test]
fn a_manifest_naming_another_pup_is_refused() {
    let store = SyntheticStore::new("id_manifest_pup");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    let other = "a".repeat(64);
    write_manifest(&store, "4.93", "4.93", &other);
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let msg = compose_err(&store, &title, &vfs);
    assert!(
        msg.contains(&other) && msg.contains(&firmware_pup_sha256()),
        "the refusal names both digests: {msg}"
    );
}

#[test]
fn a_firmware_free_boot_names_no_firmware_half() {
    let store = SyntheticStore::new("id_no_firmware");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let c = compose_boot(&ComposeInputs {
        title: &title,
        vfs_root: &vfs,
        install_root: store.root(),
        fw: None,
        game_ver: None,
        firmware_dir: None,
        no_firmware: true,
    })
    .expect("composes");
    assert!(c.identity.firmware.is_none());
    assert!(c.identity.game.is_some());
}

#[test]
fn a_firmware_executable_names_no_game_half() {
    let store = SyntheticStore::new("id_fw_exec");
    store.add_firmware("4.93", true);
    let title = manifest(
        "NPAA00001",
        GameSource::FirmwareExec {
            dir: std::path::PathBuf::from("dev_flash/vsh/module"),
        },
    );
    let vfs = store.root().join("dev_hdd0");
    let id = compose(&title, &store, &vfs, None, None).identity;
    assert!(
        id.game.is_none(),
        "a title that ships inside the firmware has no version axis of its own"
    );
    let fw = id
        .firmware
        .expect("the firmware half still names the entry");
    assert_eq!(fw.version, "4.93");
}

/// The identity names the base by the version the store selects it by,
/// so a `--game-ver base` cell and the run it records agree.
#[test]
fn the_identity_base_version_is_the_store_base_version() {
    assert_eq!(
        cellgov_compare::BASE_VERSION,
        cellgov_install::store::BASE_GAME_VER
    );
}
