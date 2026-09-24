use super::*;
use cellgov_boot::compose::{compose_boot, ComposeInputs};
use cellgov_boot::manifest::{CheckpointTrigger, Distribution, GameSource};
use cellgov_testkit::store::SyntheticStore;

fn manifest(content_id: &str, source: GameSource) -> TitleManifest {
    TitleManifest {
        content_id: content_id.to_string(),
        short_name: "synthetic".to_string(),
        display_name: "Synthetic Title".to_string(),
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
    store: &SyntheticStore,
    title: &TitleManifest,
    game_ver: Option<&str>,
) -> BootComposition {
    let vfs = store.root().join("dev_hdd0");
    compose_boot(&ComposeInputs {
        title,
        vfs_root: &vfs,
        install_root: store.root(),
        fw: None,
        game_ver,
        firmware_dir: None,
        no_firmware: false,
    })
    .unwrap()
}

#[test]
fn the_banner_says_the_firmware_shipped_with_the_disc() {
    let store = SyntheticStore::new("ban_shipped");
    store.add_firmware("3.55", true);
    store.add_firmware("4.91", true);
    store.add_disc_base_shipping("BLAA00001", "01.00", "3.55");
    let title = manifest("BLAA00001", GameSource::Disc);
    let lines = render(&title, &compose(&store, &title, None));
    assert!(lines[2].starts_with("firmware 3.55"), "got: {}", lines[2]);
    assert!(
        lines[2].contains("(shipped with this disc;"),
        "got: {}",
        lines[2]
    );
    let vfs = store.root().join("dev_hdd0");
    let named = compose_boot(&ComposeInputs {
        title: &title,
        vfs_root: &vfs,
        install_root: store.root(),
        fw: Some("4.91"),
        game_ver: None,
        firmware_dir: None,
        no_firmware: false,
    })
    .unwrap();
    let lines = render(&title, &named);
    assert!(lines[2].contains("(--fw;"), "got: {}", lines[2]);
}

#[test]
fn the_banner_names_the_title_the_version_and_the_firmware() {
    let store = SyntheticStore::new("ban_base");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    let title = manifest("NPAA00001", GameSource::Hdd);
    let lines = render(&title, &compose(&store, &title, None));
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("synthetic"), "got: {}", lines[0]);
    assert!(lines[0].contains("NPAA00001"), "got: {}", lines[0]);
    assert!(lines[0].contains("Synthetic Title"), "got: {}", lines[0]);
    assert!(lines[1].starts_with("game     base"), "got: {}", lines[1]);
    assert!(lines[1].contains("psn-hdd"), "got: {}", lines[1]);
    assert!(lines[2].starts_with("firmware 4.93"), "got: {}", lines[2]);
    assert!(lines[2].contains("ffff..ffff"), "got: {}", lines[2]);
}

#[test]
fn a_selected_update_names_the_base_it_sits_over() {
    let store = SyntheticStore::new("ban_update");
    store.add_firmware("4.93", true);
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    let title = manifest("NPAA00001", GameSource::Hdd);
    let lines = render(&title, &compose(&store, &title, Some("02.51")));
    assert!(
        lines[1].starts_with("game     update 02.51"),
        "got: {}",
        lines[1]
    );
    assert!(lines[1].contains("base 01.00"), "got: {}", lines[1]);
    assert!(lines[1].contains("cccc..cccc"), "got: {}", lines[1]);
}

#[test]
fn an_unmanaged_firmware_is_marked_as_such() {
    let store = SyntheticStore::new("ban_unmanaged");
    store.add_firmware("4.93", true);
    let raw = store.root().join("raw_external");
    std::fs::create_dir_all(&raw).unwrap();
    let title = manifest(
        "VSH",
        GameSource::FirmwareExec {
            dir: store.firmware_entry("4.93").join("dev_flash/vsh/module"),
        },
    );
    let vfs = store.root().join("dev_hdd0");
    let composition = compose_boot(&ComposeInputs {
        title: &title,
        vfs_root: &vfs,
        install_root: store.root(),
        fw: None,
        game_ver: None,
        firmware_dir: Some(&raw),
        no_firmware: false,
    })
    .unwrap();
    let lines = render(&title, &composition);
    assert!(
        lines[2].starts_with("firmware unmanaged"),
        "got: {}",
        lines[2]
    );
    assert!(lines[1].contains("unmanaged"), "got: {}", lines[1]);
}

#[test]
fn a_shortfall_note_names_the_entry_and_both_versions() {
    let notes = vec![UnderstatedFirmware {
        entry: GameVersion::Update("02.51".to_string()),
        declared: "04.5300".to_string(),
        selected: "3.55".to_string(),
        incomparable: false,
    }];
    let lines = render_firmware_notes(&notes);
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with("warning: update 02.51 "),
        "got: {}",
        lines[0]
    );
    assert!(lines[0].contains("04.5300"), "got: {}", lines[0]);
    assert!(lines[0].contains("3.55"), "got: {}", lines[0]);
}

#[test]
fn a_base_shortfall_note_names_the_base() {
    let store = SyntheticStore::new("ban_base_shortfall");
    store.add_firmware("1.00", true);
    store.add_base_declaring("NPAA00001", "01.00", false, "03.4000");
    let title = manifest("NPAA00001", GameSource::Hdd);
    let composition = compose(&store, &title, None);
    assert_eq!(
        render(&title, &composition).len(),
        3,
        "the banner itself is unchanged"
    );
    let lines = render_firmware_notes(&composition.understated_firmware);
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with("warning: base declares system version 03.4000"),
        "got: {}",
        lines[0]
    );
    assert!(lines[0].contains("1.00"), "got: {}", lines[0]);
}

#[test]
fn an_uncomparable_note_says_so_rather_than_claiming_an_order() {
    let notes = vec![UnderstatedFirmware {
        entry: GameVersion::Base,
        declared: "latest".to_string(),
        selected: "3.55".to_string(),
        incomparable: true,
    }];
    let lines = render_firmware_notes(&notes);
    assert!(lines[0].starts_with("warning: base "), "got: {}", lines[0]);
    assert!(lines[0].contains("does not compare"), "got: {}", lines[0]);
}
