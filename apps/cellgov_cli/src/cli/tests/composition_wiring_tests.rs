//! Boot-family use of a resolved composition, and of the selection
//! flags it reads from the command line.

use std::path::PathBuf;

use super::*;
use crate::composition::compose::GameChoice;
use crate::composition::inventory::FirmwareEntry;

fn sv(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_string()).collect()
}

fn managed(entry_dir: &str) -> BootComposition {
    BootComposition {
        firmware: FirmwareChoice::Managed(FirmwareEntry {
            version: "4.93".to_string(),
            entry_dir: PathBuf::from(entry_dir),
            pup_sha256: "0".repeat(64),
        }),
        game: GameChoice::Unstored,
        mounts: Vec::new(),
        eboot_dirs: Vec::new(),
        understated_firmware: Vec::new(),
    }
}

#[test]
fn a_managed_firmware_resolves_its_modules_under_the_entrys_dev_flash() {
    let dir = firmware_module_dir(&managed("store/firmware/4.93")).expect("managed has modules");
    assert_eq!(
        PathBuf::from(dir),
        PathBuf::from("store/firmware/4.93")
            .join("dev_flash")
            .join("sys")
            .join("external"),
    );
}

#[test]
fn an_unmanaged_tree_is_passed_through_as_the_module_directory() {
    let mut composition = managed("unused");
    composition.firmware = FirmwareChoice::Unmanaged {
        dir: PathBuf::from("elsewhere/sys/external"),
    };
    assert_eq!(
        firmware_module_dir(&composition).as_deref(),
        Some("elsewhere/sys/external")
    );
}

#[test]
fn a_firmware_free_boot_names_no_module_directory() {
    let mut composition = managed("unused");
    composition.firmware = FirmwareChoice::None;
    assert_eq!(firmware_module_dir(&composition), None);
}

#[test]
fn the_selection_capture_carries_every_flag_a_child_re_resolves_from() {
    let args = sv(&[
        "cli",
        "bench-boot",
        "--title",
        "synthetic",
        "--vfs-root",
        "elsewhere/dev_hdd0",
        "--fw",
        "4.91",
        "--game-ver",
        "02.51",
    ]);
    let owned = selection_args(&args);
    let selection = owned.as_args();
    assert_eq!(selection.vfs_root, Some("elsewhere/dev_hdd0"));
    assert_eq!(selection.fw, Some("4.91"));
    assert_eq!(selection.game_ver, Some("02.51"));
    assert_eq!(selection.firmware_dir, None);
}

#[test]
fn an_invocation_naming_no_selection_flag_captures_none_of_them() {
    let args = sv(&["cli", "bench-boot", "--title", "synthetic"]);
    let owned = selection_args(&args);
    let selection = owned.as_args();
    assert_eq!(selection.vfs_root, None);
    assert_eq!(selection.fw, None);
    assert_eq!(selection.game_ver, None);
    assert_eq!(selection.firmware_dir, None);
}

/// `args_tests.rs` covers the refusal that `require_at_most_one`
/// itself performs.
#[test]
fn both_firmware_selectors_are_refused_together() {
    assert!(FIRMWARE_SELECTORS.contains(&"--fw"));
    assert!(FIRMWARE_SELECTORS.contains(&"--firmware-dir"));
}
