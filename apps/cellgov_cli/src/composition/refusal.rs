//! The boot command's words for a composition refusal.
//!
//! `cellgov_boot::compose` and `cellgov_install::store::select` refuse
//! with typed variants that name no flag. This module words each one
//! around the `--fw`, `--game-ver` and `--firmware-dir` flags the
//! selections came from, and the commands that repair the store.

use cellgov_boot::compose::ComposeError as Composition;
use cellgov_install::store::select::{
    render_list, FirmwareSelectError, FirmwareSelectedBy, GameVersionSelectError,
};

use crate::env_vars::NO_FIRMWARE_DIR;

/// Why a boot could not be composed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ComposeError {
    /// `--firmware-dir` does not name a readable module tree.
    #[error("--firmware-dir: {path} is not an existing directory")]
    FirmwareDirectory {
        /// The supplied host path.
        path: String,
    },
    /// A child run could not render the composed identity.
    #[error("serializing the run identity: {message}")]
    IdentityRender {
        /// The rendering refusal.
        message: String,
    },
    /// The store refused the composition.
    #[error("{}", composition_refusal(.0))]
    Compose(#[from] Composition),
}

/// A composition refusal, worded around the flags that asked for it.
fn composition_refusal(error: &Composition) -> String {
    match error {
        Composition::Firmware(error) => firmware_refusal(error),
        Composition::GameVersion(error) => game_version_refusal(error),
        Composition::GameVersionForFirmwareExec { short_name } => format!(
            "--game-ver does not apply to {short_name}: it ships inside the firmware, so its \
             version axis is the firmware's -- select it with --fw"
        ),
        Composition::TitleNotInStore { title_id, root } => format!(
            "--game-ver names an installed version, and {title_id} has no store entry under \
             {root}; install it first, or drop the flag"
        ),
        Composition::FirmwareRelativeWithoutEntry { short_name, dir } => format!(
            "{short_name} names its executable at {dir}, relative to a firmware entry, and this \
             run selected no managed firmware. Pick one with --fw; --firmware-dir names a module \
             directory, which is not the entry root this path is relative to"
        ),
        other => other.to_string(),
    }
}

/// A firmware-selection refusal, worded around `--fw`.
pub(crate) fn firmware_refusal(error: &FirmwareSelectError) -> String {
    match error {
        FirmwareSelectError::NotInstalled {
            asked,
            root,
            installed,
        } => format!(
            "--fw {asked:?} is not installed under {root}; installed: {}",
            render_list(installed)
        ),
        FirmwareSelectError::NoneInstalled { root } => format!(
            "no firmware is installed under {root}, and no record names one this title shipped \
             with; install one with `cellgov firmware install <PS3UPDAT.PUP>`, name a tree with \
             --firmware-dir, or set {NO_FIRMWARE_DIR}=1 to boot with no firmware at all (every \
             import then answers through the unresolved-import trampoline)"
        ),
        // The disc's tree is still installed: its record is what named
        // the version. A plain reinstall then refuses with the
        // target-exists error before it registers the disc's package;
        // `--force` reaches it (`install_iso`).
        FirmwareSelectError::ShippedNotInstalled {
            version,
            root,
            installed,
        } => format!(
            "firmware {version} shipped with this disc and is recorded on its title, but is not \
             installed under {root}; installed: {}. Reinstall the disc with \
             `cellgov title install --force <ISO>`, or install it with \
             `cellgov firmware install <PS3UPDAT.PUP>`{}",
            render_list(installed),
            fw_alternative(installed)
        ),
        FirmwareSelectError::Ambiguous { root, installed } => format!(
            "{} firmware versions are installed under {root} ({}); name the one to boot against \
             with --fw",
            installed.len(),
            render_list(installed)
        ),
        FirmwareSelectError::TreeMissing { version, root, dir } => format!(
            "firmware {version} is recorded under {root} but its tree at {dir} is missing; \
             reinstall it, or name a tree with --firmware-dir"
        ),
        FirmwareSelectError::TreeUnreadable { .. } => error.to_string(),
    }
}

/// A game-version refusal, worded around `--game-ver`.
fn game_version_refusal(error: &GameVersionSelectError) -> String {
    match error {
        GameVersionSelectError::NotInstalled {
            asked,
            title_id,
            installed,
        } => format!(
            "--game-ver {asked:?} is not installed for {title_id}; installed: {}",
            render_list(installed)
        ),
        GameVersionSelectError::Ambiguous {
            title_id,
            installed,
        } => format!(
            "{title_id} has {} versions installed ({}); name the one to boot with --game-ver",
            installed.len(),
            render_list(installed)
        ),
        GameVersionSelectError::OrphanUpdates { title_id, updates } => format!(
            "{title_id} has update(s) {} installed but no base; an update tree patches a base \
             and cannot be composed alone. Install the base with \
             `cellgov title install <PKG|ISO>`",
            render_list(updates)
        ),
    }
}

/// The `--fw` hint in a shipped-version refusal; empty when the store
/// holds nothing for the flag to name.
fn fw_alternative(installed: &[String]) -> &'static str {
    if installed.is_empty() {
        ""
    } else {
        "; --fw boots another installed version instead"
    }
}

/// What selected the firmware, as the banner names it.
pub(crate) const fn selected_by_label(selected_by: FirmwareSelectedBy) -> &'static str {
    match selected_by {
        FirmwareSelectedBy::Named => "--fw",
        FirmwareSelectedBy::Shipped => "shipped with this disc",
        FirmwareSelectedBy::Sole => "the only one installed",
    }
}

#[cfg(test)]
#[path = "tests/refusal_tests.rs"]
mod tests;
