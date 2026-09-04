//! Which firmware the runner ran when it produced a capture.
//!
//! The runner states that in its own installation: a VFS mapping for
//! the location of its `dev_flash`, and the console's own version file
//! inside that tree. This module reads both.

use std::path::{Path, PathBuf};

use cellgov_ps3_abi::dev_flash::{FLASH_MOUNT, VERSION_TXT_COMPONENTS};

/// The runner's per-installation settings directory.
const CONFIG_DIR: &str = "config";

/// Directory whose presence beside the executable moves the whole
/// settings root inside it.
const PORTABLE_DIR: &str = "portable";

/// Where the runner records its guest-path mappings.
const VFS_CONFIG_FILE: &str = "vfs.yml";

/// The mapping key naming the tree that answers `/dev_flash/`.
const DEV_FLASH_KEY: &str = "/dev_flash/";

/// The mapping key naming the directory the other keys resolve
/// relative to.
const EMULATOR_DIR_KEY: &str = "$(EmulatorDir)";

/// Why a read of the runner's firmware version fails.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RunnerFirmwareError {
    /// The mapping file exists and the read of it fails. An absent file
    /// is not an error: it leaves every mapping at its default.
    #[error("read {}: {source}", path.display())]
    VfsUnreadable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The version file inside the mapped tree is absent or unreadable.
    #[error(
        "read {}: {source}. Nothing under {} states which firmware library the capture ran; \
         install the firmware this cell names into the runner and re-capture",
        path.display(), dev_flash.display()
    )]
    VersionUnreadable {
        path: PathBuf,
        dev_flash: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The version file carries no `release` record.
    #[error("{} states no firmware version", path.display())]
    VersionUnparseable { path: PathBuf },
}

/// The firmware version installed in the runner rooted at `install_dir`.
///
/// # Errors
///
/// [`RunnerFirmwareError`] when:
///
/// - the read of the mapping file fails, or
/// - the tree it names holds no readable version.
pub(crate) fn firmware_version(install_dir: &Path) -> Result<String, RunnerFirmwareError> {
    let dev_flash = dev_flash_dir(install_dir)?;
    let mut path = dev_flash.clone();
    for c in VERSION_TXT_COMPONENTS {
        path.push(c);
    }
    let text = std::fs::read_to_string(&path).map_err(|source| {
        RunnerFirmwareError::VersionUnreadable {
            path: path.clone(),
            dev_flash,
            source,
        }
    })?;
    cellgov_ps3_abi::dev_flash::parse_version_txt(&text)
        .ok_or(RunnerFirmwareError::VersionUnparseable { path })
}

/// The directory the runner keeps its settings under.
///
/// A [`PORTABLE_DIR`] directory beside the executable becomes that
/// root, which moves both the mapping file and the directory an unset
/// [`EMULATOR_DIR_KEY`] resolves to.
// RPCS3 `Utilities/File.cpp` `fs::get_config_dir`: a `portable/`
// directory beside the executable becomes the settings root, and the
// Windows build appends `config/` to it for the settings files.
fn settings_root(install_dir: &Path) -> PathBuf {
    let portable = install_dir.join(PORTABLE_DIR);
    if portable.is_dir() {
        portable
    } else {
        install_dir.to_path_buf()
    }
}

/// The tree the runner mounts at `/dev_flash/`.
// RPCS3 `Emu/vfs_config.cpp`: `cfg_vfs::load` uses the built-in
// defaults when the file is absent. `cfg_vfs::get` substitutes a key's
// default for an empty value, then expands `$(EmulatorDir)`, which is
// the settings root when unset.
fn dev_flash_dir(install_dir: &Path) -> Result<PathBuf, RunnerFirmwareError> {
    let root = settings_root(install_dir);
    let path = root.join(CONFIG_DIR).join(VFS_CONFIG_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(source) => return Err(RunnerFirmwareError::VfsUnreadable { path, source }),
    };
    let emulator_dir = mapping(&text, EMULATOR_DIR_KEY).map_or(root, PathBuf::from);
    Ok(match mapping(&text, DEV_FLASH_KEY) {
        Some(v) => resolve(&v, &emulator_dir),
        None => emulator_dir.join(FLASH_MOUNT),
    })
}

/// The value of one top-level `<key>: <value>` mapping, or `None` when
/// the file states no value for it.
///
/// The mappings are a flat block of scalars, so a line scan reads them
/// without a YAML parser.
fn mapping(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line.trim_end().strip_prefix(key)?.strip_prefix(':')?;
        let value = rest.trim().trim_matches('"');
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// Expand a mapping's leading [`EMULATOR_DIR_KEY`] token.
// RPCS3 `Emu/vfs_config.cpp` `cfg_vfs::get` substitutes the token
// wherever it appears and gives the expansion a trailing separator. The
// join below is that separator. Only the leading token is a form the
// runner itself writes.
fn resolve(value: &str, emulator_dir: &Path) -> PathBuf {
    match value.strip_prefix(EMULATOR_DIR_KEY) {
        Some(tail) => emulator_dir.join(tail.trim_start_matches(['/', '\\'])),
        None => PathBuf::from(value),
    }
}

#[cfg(test)]
#[path = "tests/runner_firmware_tests.rs"]
mod tests;
