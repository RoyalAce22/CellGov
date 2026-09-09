//! A disc title's recorded shipped firmware as the default `--fw`.

use super::*;
use crate::composition::banner::render;
use crate::composition::select::{FirmwareSelectedBy, ManagedFirmware};
use crate::composition::test_support::SyntheticStore;
use crate::game::manifest::{CheckpointTrigger, Distribution};

const DISABLE_ENV: &str = "CELLGOV_NO_FIRMWARE_DIR";

const DISC: &str = "BLAA00001";

fn disc_manifest() -> TitleManifest {
    TitleManifest {
        content_id: DISC.to_string(),
        short_name: "t".to_string(),
        display_name: "Synthetic".to_string(),
        eboot_candidates: vec!["EBOOT.BIN".to_string()],
        year: 2007,
        developer: "test-developer".to_string(),
        engine: "test-engine".to_string(),
        distribution: Distribution::DiscIso,
        rap_filename: None,
        bench_max_steps: None,
        system_ver: None,
        checkpoint: CheckpointTrigger::ProcessExit,
        source: GameSource::Disc,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
        matrix: Vec::new(),
    }
}

fn compose(
    store: &SyntheticStore,
    title: &TitleManifest,
    fw: Option<&str>,
    game_ver: Option<&str>,
) -> Result<BootComposition, ComposeError> {
    let vfs = store.root().join("dev_hdd0");
    compose_boot(&ComposeInputs {
        title,
        vfs_root: &vfs,
        install_root: store.root(),
        fw,
        game_ver,
        firmware_dir: None,
        no_firmware: false,
        disable_env: DISABLE_ENV,
    })
}

fn managed(c: &BootComposition) -> &ManagedFirmware {
    match &c.firmware {
        FirmwareChoice::Managed(m) => m,
        other => panic!("expected a managed firmware, got {other:?}"),
    }
}

fn firmware_refusal(err: ComposeError) -> FirmwareSelectError {
    match err {
        ComposeError::Firmware(e) => e,
        other => panic!("expected a firmware refusal, got {other}"),
    }
}

#[test]
fn a_disc_title_boots_the_firmware_it_shipped_whatever_else_is_installed() {
    let store = SyntheticStore::new("shipped_default");
    store.add_firmware("3.55", true);
    store.add_firmware("4.91", true);
    store.add_disc_base_shipping(DISC, "01.00", "3.55");
    let c = compose(&store, &disc_manifest(), None, None).unwrap();
    let selected = managed(&c);
    assert_eq!(selected.entry.version, "3.55");
    assert_eq!(selected.selected_by, FirmwareSelectedBy::Shipped);
    let flash = c
        .mounts
        .iter()
        .find(|m| m.prefix == "/dev_flash")
        .expect("a managed firmware answers /dev_flash");
    assert_eq!(flash.roots, vec![store.firmware_dev_flash("3.55")]);
}

#[test]
fn an_implicit_choice_is_stamped_on_the_identity_like_a_named_one() {
    let store = SyntheticStore::new("shipped_identity");
    store.add_firmware("3.55", true);
    store.add_firmware("4.91", true);
    store.add_disc_base_shipping(DISC, "01.00", "3.55");
    let implicit = compose(&store, &disc_manifest(), None, None).unwrap();
    let named = compose(&store, &disc_manifest(), Some("3.55"), None).unwrap();
    let stamped = implicit
        .identity
        .firmware
        .as_ref()
        .expect("a managed firmware is stamped");
    assert_eq!(stamped.version, "3.55");
    assert_eq!(
        implicit.identity.firmware, named.identity.firmware,
        "how the firmware was selected is not part of what the run tests"
    );
}

#[test]
fn the_flag_outranks_the_shipped_firmware() {
    let store = SyntheticStore::new("shipped_flag_wins");
    store.add_firmware("3.55", true);
    store.add_firmware("4.91", true);
    store.add_disc_base_shipping(DISC, "01.00", "3.55");
    let c = compose(&store, &disc_manifest(), Some("4.91"), None).unwrap();
    let selected = managed(&c);
    assert_eq!(selected.entry.version, "4.91");
    assert_eq!(selected.selected_by, FirmwareSelectedBy::Flag);
}

#[test]
fn a_shipped_firmware_that_is_not_installed_is_refused_by_name() {
    let store = SyntheticStore::new("shipped_missing");
    store.add_firmware("4.91", true);
    store.add_disc_base_shipping(DISC, "01.00", "3.55");
    let err = firmware_refusal(compose(&store, &disc_manifest(), None, None).unwrap_err());
    assert!(
        matches!(err, FirmwareSelectError::ShippedNotInstalled { .. }),
        "the sole installed firmware must not stand in: {err}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("firmware 3.55 shipped with this disc"),
        "got: {msg}"
    );
    assert!(msg.contains("installed: 4.91"), "got: {msg}");
    // The disc tree is still there, so a reinstall without --force
    // refuses as target-exists before it registers the disc's package.
    assert!(
        msg.contains("Reinstall the disc with `cellgov title install --force <ISO>`"),
        "got: {msg}"
    );
    assert!(msg.contains("cellgov firmware install"), "got: {msg}");
    assert!(msg.contains("--fw"), "got: {msg}");
}

#[test]
fn a_shipped_firmware_with_nothing_installed_is_refused_by_name_not_by_count() {
    let store = SyntheticStore::new("shipped_none_at_all");
    store.add_disc_base_shipping(DISC, "01.00", "3.55");
    let err = firmware_refusal(compose(&store, &disc_manifest(), None, None).unwrap_err());
    assert!(
        matches!(err, FirmwareSelectError::ShippedNotInstalled { .. }),
        "the empty count must not answer for a recorded version: {err}"
    );
    let msg = err.to_string();
    assert!(msg.contains("installed: (none)"), "got: {msg}");
    assert!(
        !msg.contains("--fw"),
        "nothing is installed for --fw to name: {msg}"
    );
}

#[test]
fn the_flag_still_boots_another_version_when_the_shipped_one_is_gone() {
    let store = SyntheticStore::new("shipped_missing_flag");
    store.add_firmware("4.91", true);
    store.add_disc_base_shipping(DISC, "01.00", "3.55");
    let c = compose(&store, &disc_manifest(), Some("4.91"), None).unwrap();
    assert_eq!(managed(&c).entry.version, "4.91");
}

#[test]
fn a_shipped_firmware_whose_tree_is_gone_is_refused_like_any_other() {
    let store = SyntheticStore::new("shipped_tree_gone");
    store.add_firmware("3.55", false);
    store.add_disc_base_shipping(DISC, "01.00", "3.55");
    let err = firmware_refusal(compose(&store, &disc_manifest(), None, None).unwrap_err());
    assert!(
        matches!(err, FirmwareSelectError::TreeMissing { .. }),
        "got: {err}"
    );
}

#[test]
fn a_disc_whose_record_names_no_shipped_firmware_takes_the_count() {
    let store = SyntheticStore::new("shipped_unrecorded");
    store.add_firmware("3.55", true);
    store.add_firmware("4.91", true);
    store.add_base(DISC, "01.00", true);
    let err = firmware_refusal(compose(&store, &disc_manifest(), None, None).unwrap_err());
    assert!(
        matches!(err, FirmwareSelectError::Ambiguous { .. }),
        "got: {err}"
    );

    let store = SyntheticStore::new("shipped_unrecorded_sole");
    store.add_firmware("4.91", true);
    store.add_base(DISC, "01.00", true);
    let c = compose(&store, &disc_manifest(), None, None).unwrap();
    assert_eq!(managed(&c).selected_by, FirmwareSelectedBy::Sole);
}

#[test]
fn nothing_installed_says_the_record_names_no_shipped_firmware() {
    let store = SyntheticStore::new("shipped_none_installed");
    store.add_base(DISC, "01.00", true);
    let err = firmware_refusal(compose(&store, &disc_manifest(), None, None).unwrap_err());
    let msg = err.to_string();
    assert!(msg.starts_with("no firmware is installed"), "got: {msg}");
    assert!(
        msg.contains("no record names one this title shipped with"),
        "got: {msg}"
    );
    assert!(msg.contains("cellgov firmware install"), "got: {msg}");
}

#[test]
fn the_banner_says_the_firmware_shipped_with_the_disc() {
    let store = SyntheticStore::new("shipped_banner");
    store.add_firmware("3.55", true);
    store.add_firmware("4.91", true);
    store.add_disc_base_shipping(DISC, "01.00", "3.55");
    let title = disc_manifest();
    let lines = render(&title, &compose(&store, &title, None, None).unwrap());
    assert!(lines[2].starts_with("firmware 3.55"), "got: {}", lines[2]);
    assert!(
        lines[2].contains("(shipped with this disc;"),
        "got: {}",
        lines[2]
    );
    let lines = render(
        &title,
        &compose(&store, &title, Some("4.91"), None).unwrap(),
    );
    assert!(lines[2].contains("(--fw;"), "got: {}", lines[2]);
}

#[test]
fn an_update_over_a_disc_base_boots_the_shipped_firmware_and_warns_when_it_declares_more() {
    let store = SyntheticStore::new("shipped_update");
    store.add_firmware("3.55", true);
    store.add_firmware("4.91", true);
    store.add_disc_base_shipping(DISC, "01.00", "3.55");
    store.add_update_declaring(DISC, "02.51", None, "04.0000");
    let c = compose(&store, &disc_manifest(), None, Some("02.51")).unwrap();
    assert_eq!(managed(&c).entry.version, "3.55");
    assert_eq!(c.understated_firmware.len(), 1);
    assert_eq!(
        c.understated_firmware[0].entry,
        GameVersion::Update("02.51".to_string())
    );
    assert_eq!(c.understated_firmware[0].selected, "3.55");
}

/// `compose` with the two selections that bypass the store's firmware
/// axis: `--firmware-dir`, and the no-firmware variable.
fn compose_overriding(
    store: &SyntheticStore,
    title: &TitleManifest,
    firmware_dir: Option<&Path>,
    no_firmware: bool,
) -> Result<BootComposition, ComposeError> {
    let vfs = store.root().join("dev_hdd0");
    compose_boot(&ComposeInputs {
        title,
        vfs_root: &vfs,
        install_root: store.root(),
        fw: None,
        game_ver: None,
        firmware_dir,
        no_firmware,
        disable_env: DISABLE_ENV,
    })
}

#[test]
fn firmware_dir_outranks_the_shipped_record_and_carries_no_version() {
    let store = SyntheticStore::new("shipped_firmware_dir");
    store.add_firmware("3.55", true);
    store.add_disc_base_shipping(DISC, "01.00", "3.55");
    let external = store.root().join("external");
    std::fs::create_dir_all(&external).unwrap();
    let c = compose_overriding(&store, &disc_manifest(), Some(&external), false).unwrap();
    assert_eq!(c.firmware, FirmwareChoice::Unmanaged { dir: external });
    assert!(
        c.mounts.iter().all(|m| m.prefix != "/dev_flash"),
        "an unmanaged tree is not a store mount: {:?}",
        c.mounts
    );
    assert_eq!(c.identity.firmware, None);
}

#[test]
fn firmware_dir_does_not_consult_the_shipped_record_at_all() {
    // The recorded version is not installed, which refuses a managed
    // boot; an unmanaged tree never asks the store.
    let store = SyntheticStore::new("shipped_firmware_dir_missing");
    store.add_disc_base_shipping(DISC, "01.00", "3.55");
    let external = store.root().join("external");
    std::fs::create_dir_all(&external).unwrap();
    let c = compose_overriding(&store, &disc_manifest(), Some(&external), false).unwrap();
    assert!(matches!(c.firmware, FirmwareChoice::Unmanaged { .. }));
}

#[test]
fn the_no_firmware_variable_outranks_the_shipped_record() {
    let store = SyntheticStore::new("shipped_no_firmware");
    store.add_disc_base_shipping(DISC, "01.00", "3.55");
    let c = compose_overriding(&store, &disc_manifest(), None, true).unwrap();
    assert_eq!(c.firmware, FirmwareChoice::None);
    assert!(c.mounts.iter().all(|m| m.prefix != "/dev_flash"));
    assert_eq!(c.identity.firmware, None);
}

#[test]
fn an_orphan_has_no_base_to_name_a_shipped_firmware_and_takes_the_count() {
    let store = SyntheticStore::new("shipped_orphan");
    store.add_firmware("4.91", true);
    store.add_update(DISC, "02.51");
    let err = compose(&store, &disc_manifest(), None, None).unwrap_err();
    assert!(
        matches!(
            err,
            ComposeError::GameVersion(GameVersionSelectError::OrphanUpdates { .. })
        ),
        "the sole firmware selects and the orphan refuses on its own axis: {err}"
    );
}
