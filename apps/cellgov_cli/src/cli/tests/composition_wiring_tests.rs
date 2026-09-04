//! Boot-family use of a resolved composition, and of the selection
//! flags it reads from the command line.

use std::path::PathBuf;

use super::*;
use crate::composition::compose::GameChoice;
use crate::composition::inventory::FirmwareEntry;

fn selection(fw: Option<&str>, game_ver: Option<&str>, dir: Option<&str>) -> BootSelection {
    BootSelection {
        fw: fw.map(str::to_string),
        game_ver: game_ver.map(str::to_string),
        firmware_dir: dir.map(PathBuf::from),
    }
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
        identity: cellgov_compare::RunIdentity::default(),
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
    let owned = selection_args(
        &selection(Some("4.91"), Some("02.51"), None),
        Some(Path::new("elsewhere/dev_hdd0")),
    );
    let selection = owned.as_args();
    assert_eq!(selection.vfs_root, Some("elsewhere/dev_hdd0"));
    assert_eq!(selection.fw, Some("4.91"));
    assert_eq!(selection.game_ver, Some("02.51"));
    assert_eq!(selection.firmware_dir, None);
}

#[test]
fn an_invocation_naming_no_selection_flag_captures_none_of_them() {
    let owned = selection_args(&selection(None, None, None), None);
    let selection = owned.as_args();
    assert_eq!(selection.vfs_root, None);
    assert_eq!(selection.fw, None);
    assert_eq!(selection.game_ver, None);
    assert_eq!(selection.firmware_dir, None);
}
