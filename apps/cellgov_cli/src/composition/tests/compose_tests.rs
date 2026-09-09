use super::*;
use crate::composition::test_support::SyntheticStore;
use crate::game::manifest::{CheckpointTrigger, Distribution};

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

fn inputs<'a>(
    title: &'a TitleManifest,
    store: &'a SyntheticStore,
    vfs_root: &'a Path,
) -> ComposeInputs<'a> {
    ComposeInputs {
        title,
        vfs_root,
        install_root: store.root(),
        fw: None,
        game_ver: None,
        firmware_dir: None,
        no_firmware: false,
        disable_env: DISABLE_ENV,
    }
}

fn mount<'a>(c: &'a BootComposition, prefix: &str) -> &'a ComposedMount {
    c.mounts
        .iter()
        .find(|m| m.prefix == prefix)
        .unwrap_or_else(|| panic!("no {prefix} mount in {:?}", c.mounts))
}

#[test]
fn firmware_answers_dev_flash_from_the_selected_entry() {
    let store = SyntheticStore::new("cmp_fw");
    store.add_firmware("4.93", true);
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let c = compose_boot(&inputs(&title, &store, &vfs)).unwrap();
    assert_eq!(
        mount(&c, "/dev_flash").roots,
        vec![store.firmware_dev_flash("4.93")]
    );
}

#[test]
fn an_hdd_base_alone_answers_its_game_mount() {
    let store = SyntheticStore::new("cmp_hdd_base");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let c = compose_boot(&inputs(&title, &store, &vfs)).unwrap();
    let base = store.root().join("titles/NPAA00001/base/game");
    assert_eq!(
        mount(&c, "/dev_hdd0/game/NPAA00001").roots,
        vec![base.clone()]
    );
    assert_eq!(c.eboot_dirs, vec![base.join("USRDIR")]);
}

#[test]
fn an_hdd_update_shadows_the_base_it_patches() {
    let store = SyntheticStore::new("cmp_hdd_update");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let mut i = inputs(&title, &store, &vfs);
    i.game_ver = Some("02.51");
    let c = compose_boot(&i).unwrap();
    let update = store.root().join("titles/NPAA00001/updates/02.51/game");
    let base = store.root().join("titles/NPAA00001/base/game");
    assert_eq!(
        mount(&c, "/dev_hdd0/game/NPAA00001").roots,
        vec![update.clone(), base.clone()]
    );
    assert_eq!(
        c.eboot_dirs,
        vec![update.join("USRDIR"), base.join("USRDIR")]
    );
}

#[test]
fn a_disc_base_keeps_its_bdvd_mount_and_the_update_answers_the_game_mount() {
    let store = SyntheticStore::new("cmp_disc_update");
    store.add_firmware("4.93", true);
    store.add_base("BLAA00001", "02.00", true);
    store.add_update("BLAA00001", "02.51");
    let title = manifest("BLAA00001", GameSource::Disc);
    let vfs = store.root().join("dev_hdd0");
    let mut i = inputs(&title, &store, &vfs);
    i.game_ver = Some("02.51");
    let c = compose_boot(&i).unwrap();
    let disc = store.root().join("titles/BLAA00001/base/disc");
    let update = store.root().join("titles/BLAA00001/updates/02.51/game");
    assert_eq!(mount(&c, "/dev_bdvd/BLAA00001").roots, vec![disc.clone()]);
    assert_eq!(
        mount(&c, "/dev_hdd0/game/BLAA00001").roots,
        vec![update.clone()]
    );
    assert_eq!(
        c.eboot_dirs,
        vec![update.join("USRDIR"), disc.join("PS3_GAME").join("USRDIR")]
    );
}

#[test]
fn a_title_with_no_store_entry_composes_no_title_mount() {
    let store = SyntheticStore::new("cmp_unstored");
    store.add_firmware("4.93", true);
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let c = compose_boot(&inputs(&title, &store, &vfs)).unwrap();
    assert!(matches!(c.game, GameChoice::Unstored));
    assert!(c.mounts.iter().all(|m| m.prefix == "/dev_flash"));
    assert_eq!(c.eboot_dirs, vec![vfs.join("game/NPAA00001/USRDIR")]);
}

#[test]
fn game_ver_for_a_title_the_store_does_not_hold_is_refused() {
    let store = SyntheticStore::new("cmp_unstored_flag");
    store.add_firmware("4.93", true);
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let mut i = inputs(&title, &store, &vfs);
    i.game_ver = Some("base");
    let msg = compose_boot(&i).unwrap_err().to_string();
    assert!(msg.contains("no store entry"), "got: {msg}");
}

#[test]
fn a_firmware_exec_path_resolves_against_the_selected_firmware() {
    let store = SyntheticStore::new("cmp_fwexec");
    store.add_firmware("4.91", true);
    store.add_firmware("4.93", true);
    let title = manifest(
        "VSH",
        GameSource::FirmwareExec {
            dir: PathBuf::from("dev_flash/vsh/module"),
        },
    );
    let vfs = store.root().join("dev_hdd0");
    let mut i = inputs(&title, &store, &vfs);
    i.fw = Some("4.91");
    let c = compose_boot(&i).unwrap();
    assert_eq!(
        c.eboot_dirs,
        vec![store.firmware_entry("4.91").join("dev_flash/vsh/module")]
    );
    i.fw = Some("4.93");
    let c = compose_boot(&i).unwrap();
    assert_eq!(
        c.eboot_dirs,
        vec![store.firmware_entry("4.93").join("dev_flash/vsh/module")]
    );
}

#[test]
fn a_firmware_exec_title_refuses_game_ver() {
    let store = SyntheticStore::new("cmp_fwexec_gv");
    store.add_firmware("4.93", true);
    let title = manifest(
        "VSH",
        GameSource::FirmwareExec {
            dir: PathBuf::from("dev_flash/vsh/module"),
        },
    );
    let vfs = store.root().join("dev_hdd0");
    let mut i = inputs(&title, &store, &vfs);
    i.game_ver = Some("base");
    let msg = compose_boot(&i).unwrap_err().to_string();
    assert!(msg.contains("--game-ver does not apply"), "got: {msg}");
    assert!(msg.contains("--fw"), "got: {msg}");
}

#[test]
fn an_unmanaged_firmware_dir_composes_no_dev_flash_mount_and_keeps_an_absolute_path() {
    let store = SyntheticStore::new("cmp_unmanaged");
    store.add_firmware("4.93", true);
    let raw = store.root().join("raw_external");
    std::fs::create_dir_all(&raw).unwrap();
    let module_dir = store.firmware_entry("4.93").join("dev_flash/vsh/module");
    let title = manifest(
        "VSH",
        GameSource::FirmwareExec {
            dir: module_dir.clone(),
        },
    );
    let vfs = store.root().join("dev_hdd0");
    let mut i = inputs(&title, &store, &vfs);
    i.firmware_dir = Some(&raw);
    let c = compose_boot(&i).unwrap();
    assert!(matches!(c.firmware, FirmwareChoice::Unmanaged { .. }));
    assert!(c.mounts.iter().all(|m| m.prefix != "/dev_flash"));
    assert_eq!(c.eboot_dirs, vec![module_dir]);
    assert!(matches!(
        c.game,
        GameChoice::Firmware {
            unmanaged_path: true,
            ..
        }
    ));
}

#[test]
fn a_firmware_relative_path_with_no_managed_firmware_is_refused() {
    let store = SyntheticStore::new("cmp_unmanaged_rel");
    store.add_firmware("4.93", true);
    let raw = store.root().join("raw_external");
    std::fs::create_dir_all(&raw).unwrap();
    let title = manifest(
        "VSH",
        GameSource::FirmwareExec {
            dir: PathBuf::from("dev_flash/vsh/module"),
        },
    );
    let vfs = store.root().join("dev_hdd0");
    let mut i = inputs(&title, &store, &vfs);
    i.firmware_dir = Some(&raw);
    let msg = compose_boot(&i).unwrap_err().to_string();
    assert!(msg.contains("relative to a firmware entry"), "got: {msg}");
    assert!(msg.contains("--fw"), "got: {msg}");
}

#[test]
fn a_license_shared_by_two_titles_composes_once() {
    let store = SyntheticStore::new("cmp_exdata_ok");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    store.add_base("NPAA00002", "01.00", false);
    store.add_title_rap("NPAA00001", "shared.rap", b"0123456789abcdef");
    store.add_title_rap("NPAA00002", "shared.rap", b"0123456789abcdef");
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let c = compose_boot(&inputs(&title, &store, &vfs)).unwrap();
    assert_eq!(
        mount(&c, "/dev_hdd0/home/00000001/exdata").roots,
        vec![
            store.root().join("titles/NPAA00001/exdata"),
            store.root().join("titles/NPAA00002/exdata"),
        ]
    );
}

#[test]
fn two_different_licenses_of_one_name_are_refused_by_name() {
    let store = SyntheticStore::new("cmp_exdata_conflict");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    store.add_base("NPAA00002", "01.00", false);
    store.add_title_rap("NPAA00001", "shared.rap", b"0123456789abcdef");
    store.add_title_rap("NPAA00002", "shared.rap", b"fedcba9876543210");
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let msg = compose_boot(&inputs(&title, &store, &vfs))
        .unwrap_err()
        .to_string();
    assert!(msg.contains("shared.rap"), "got: {msg}");
    assert!(msg.contains("NPAA00002"), "got: {msg}");
}

#[test]
fn an_update_needing_newer_firmware_is_reported_not_refused() {
    let store = SyntheticStore::new("cmp_shortfall");
    store.add_firmware("3.55", true);
    store.add_base("NPAA00001", "01.00", false);
    store.add_update_needing("NPAA00001", "02.51", Some("04.5300"));
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let mut i = inputs(&title, &store, &vfs);
    i.game_ver = Some("02.51");
    let c = compose_boot(&i).unwrap();
    assert_eq!(c.understated_firmware.len(), 1);
    assert_eq!(c.understated_firmware[0].declared, "04.5300");
    assert_eq!(c.understated_firmware[0].selected, "3.55");
    assert!(!c.understated_firmware[0].incomparable);
}

#[test]
fn a_firmware_that_meets_the_declared_minimum_reports_nothing() {
    let store = SyntheticStore::new("cmp_shortfall_ok");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    store.add_update_needing("NPAA00001", "02.51", Some("04.5300"));
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let mut i = inputs(&title, &store, &vfs);
    i.game_ver = Some("02.51");
    let c = compose_boot(&i).unwrap();
    assert!(c.understated_firmware.is_empty());
}

#[test]
fn version_key_normalizes_the_two_written_forms_of_one_version() {
    assert_eq!(version_key("4.93"), version_key("04.9300"));
    assert!(version_key("4.9") < version_key("4.93"));
    assert!(version_key("04.9300") < version_key("04.9312"));
    assert_eq!(version_key("latest"), None);
    assert_eq!(version_key("4."), None);
}

#[test]
fn a_version_string_no_order_can_be_read_from_yields_no_key() {
    for malformed in ["", ".93", "4.930000", "4.9a", "99999999999.9300", "493"] {
        assert_eq!(version_key(malformed), None, "for {malformed:?}");
    }
}

#[test]
fn a_subdirectory_in_a_license_root_does_not_stop_the_union() {
    let store = SyntheticStore::new("cmp_exdata_subdir");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    store.add_title_rap("NPAA00001", "a.rap", b"0123456789abcdef");
    std::fs::create_dir_all(store.root().join("titles/NPAA00001/exdata/nested")).unwrap();
    let title = manifest("NPAA00001", GameSource::Hdd);
    let vfs = store.root().join("dev_hdd0");
    let c = compose_boot(&inputs(&title, &store, &vfs)).unwrap();
    assert_eq!(
        mount(&c, "/dev_hdd0/home/00000001/exdata").roots,
        vec![store.root().join("titles/NPAA00001/exdata")]
    );
}
