//! Every flag-worded refusal, pinned as the boot command prints it.

use super::*;

fn versions(list: &[&str]) -> Vec<String> {
    list.iter().map(ToString::to_string).collect()
}

fn firmware(error: FirmwareSelectError) -> String {
    ComposeError::Compose(Composition::Firmware(error)).to_string()
}

fn game_version(error: GameVersionSelectError) -> String {
    ComposeError::Compose(Composition::GameVersion(error)).to_string()
}

#[test]
fn a_named_firmware_that_is_not_installed_names_the_flag() {
    assert_eq!(
        firmware(FirmwareSelectError::NotInstalled {
            asked: "4.93".to_string(),
            root: "store".to_string(),
            installed: versions(&["4.91"]),
        }),
        "--fw \"4.93\" is not installed under store; installed: 4.91"
    );
}

#[test]
fn no_installed_firmware_names_the_install_command_and_the_opt_out() {
    assert_eq!(
        firmware(FirmwareSelectError::NoneInstalled {
            root: "store".to_string(),
        }),
        "no firmware is installed under store, and no record names one this title shipped with; \
         install one with `cellgov firmware install <PS3UPDAT.PUP>`, name a tree with \
         --firmware-dir, or set CELLGOV_NO_FIRMWARE_DIR=1 to boot with no firmware at all \
         (every import then answers through the unresolved-import trampoline)"
    );
}

#[test]
fn a_shipped_firmware_that_is_not_installed_names_both_repairs_and_the_flag() {
    assert_eq!(
        firmware(FirmwareSelectError::ShippedNotInstalled {
            version: "3.55".to_string(),
            root: "store".to_string(),
            installed: versions(&["4.91"]),
        }),
        "firmware 3.55 shipped with this disc and is recorded on its title, but is not \
         installed under store; installed: 4.91. Reinstall the disc with \
         `cellgov title install --force <ISO>`, or install it with \
         `cellgov firmware install <PS3UPDAT.PUP>`; --fw boots another installed version \
         instead"
    );
}

#[test]
fn a_shipped_firmware_with_nothing_installed_offers_no_flag() {
    assert_eq!(
        firmware(FirmwareSelectError::ShippedNotInstalled {
            version: "3.55".to_string(),
            root: "store".to_string(),
            installed: Vec::new(),
        }),
        "firmware 3.55 shipped with this disc and is recorded on its title, but is not \
         installed under store; installed: (none). Reinstall the disc with \
         `cellgov title install --force <ISO>`, or install it with \
         `cellgov firmware install <PS3UPDAT.PUP>`"
    );
}

#[test]
fn several_firmwares_ask_for_the_flag() {
    assert_eq!(
        firmware(FirmwareSelectError::Ambiguous {
            root: "store".to_string(),
            installed: versions(&["3.55", "4.91"]),
        }),
        "2 firmware versions are installed under store (3.55, 4.91); name the one to boot \
         against with --fw"
    );
}

#[test]
fn a_firmware_tree_that_is_gone_names_the_directory_flag() {
    assert_eq!(
        firmware(FirmwareSelectError::TreeMissing {
            version: "4.93".to_string(),
            root: "store".to_string(),
            dir: "store/firmware/4.93/dev_flash".to_string(),
        }),
        "firmware 4.93 is recorded under store but its tree at store/firmware/4.93/dev_flash is \
         missing; reinstall it, or name a tree with --firmware-dir"
    );
}

#[test]
fn an_unreadable_firmware_tree_keeps_the_library_wording() {
    assert_eq!(
        firmware(FirmwareSelectError::TreeUnreadable {
            version: "4.93".to_string(),
            root: "store".to_string(),
            dir: "d".to_string(),
            reason: "denied".to_string(),
        }),
        "firmware 4.93 is recorded under store but its tree at d could not be probed: denied"
    );
}

#[test]
fn the_game_version_refusals_name_the_flag() {
    assert_eq!(
        game_version(GameVersionSelectError::NotInstalled {
            asked: "03.00".to_string(),
            title_id: "NPAA00001".to_string(),
            installed: versions(&["base", "02.51"]),
        }),
        "--game-ver \"03.00\" is not installed for NPAA00001; installed: base, 02.51"
    );
    assert_eq!(
        game_version(GameVersionSelectError::Ambiguous {
            title_id: "NPAA00001".to_string(),
            installed: versions(&["base", "02.51"]),
        }),
        "NPAA00001 has 2 versions installed (base, 02.51); name the one to boot with --game-ver"
    );
    assert_eq!(
        game_version(GameVersionSelectError::OrphanUpdates {
            title_id: "NPAA00001".to_string(),
            updates: versions(&["02.51"]),
        }),
        "NPAA00001 has update(s) 02.51 installed but no base; an update tree patches a base \
         and cannot be composed alone. Install the base with `cellgov title install <PKG|ISO>`"
    );
}

#[test]
fn the_composition_refusals_name_their_flags() {
    let render = |error| ComposeError::Compose(error).to_string();
    assert_eq!(
        render(Composition::GameVersionForFirmwareExec {
            short_name: "vsh".to_string(),
        }),
        "--game-ver does not apply to vsh: it ships inside the firmware, so its version axis \
         is the firmware's -- select it with --fw"
    );
    assert_eq!(
        render(Composition::TitleNotInStore {
            title_id: "NPAA00001".to_string(),
            root: "store".to_string(),
        }),
        "--game-ver names an installed version, and NPAA00001 has no store entry under store; \
         install it first, or drop the flag"
    );
    assert_eq!(
        render(Composition::FirmwareRelativeWithoutEntry {
            short_name: "vsh".to_string(),
            dir: "dev_flash/vsh/module".to_string(),
        }),
        "vsh names its executable at dev_flash/vsh/module, relative to a firmware entry, and \
         this run selected no managed firmware. Pick one with --fw; --firmware-dir names a \
         module directory, which is not the entry root this path is relative to"
    );
    assert_eq!(
        render(Composition::BaseRecordMissing {
            title_id: "NPAA00001".to_string(),
        }),
        "NPAA00001 has updates in the store but no base record; reinstall the title"
    );
}

#[test]
fn the_banner_names_what_selected_the_firmware() {
    assert_eq!(selected_by_label(FirmwareSelectedBy::Named), "--fw");
    assert_eq!(
        selected_by_label(FirmwareSelectedBy::Shipped),
        "shipped with this disc"
    );
    assert_eq!(
        selected_by_label(FirmwareSelectedBy::Sole),
        "the only one installed"
    );
}
