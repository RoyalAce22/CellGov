//! Which dev_flash entries an install drops before they reach disk.

use cellgov_ps3_abi::format::dev_flash::FLASH_MOUNT;

use crate::tar;

/// dev_flash subtrees CellGov never loads and prunes at install time:
/// the PS1 / PS2 / PSP backward-compat emulators, which a CBE
/// execution oracle never runs.
const PRUNED_DEV_FLASH_DIRS: [&str; 3] = ["ps1emu/", "ps2emu/", "pspemu/"];

/// Whether an inner dev_flash entry is dropped at install time.
///
/// The prune decides on [`tar::route_entry_path`]'s output so it sees
/// the exact path the extractor would write, whatever the `000/`
/// packaging or leading slash the raw name carries.
pub(super) fn is_install_excluded(entry_name: &str) -> bool {
    let Some(routed) = tar::route_entry_path(entry_name) else {
        return false;
    };
    let Some(rel) = routed
        .strip_prefix(FLASH_MOUNT)
        .and_then(|r| r.strip_prefix('/'))
    else {
        return false;
    };
    PRUNED_DEV_FLASH_DIRS.iter().any(|d| rel.starts_with(d))
}

#[cfg(test)]
#[path = "tests/prune_tests.rs"]
mod tests;
