//! The firmware version key, read out of the extracted tree.
//!
//! `vsh/etc/version.txt` is the file the console reads, so the key a
//! store entry is named after is the same string a user sees in the
//! XMB: `release:04.9100:` becomes `4.91`.

use std::path::{Path, PathBuf};

use cellgov_ps3_abi::format::dev_flash::{parse_version_txt, VERSION_TXT_COMPONENTS};

use super::error::FirmwareInstallError;

/// Characters of the file's first line a refusal quotes.
const LEADING_SHOWN: usize = 64;

fn version_txt_path(dev_flash_dir: &Path) -> PathBuf {
    let mut p = dev_flash_dir.to_path_buf();
    for c in VERSION_TXT_COMPONENTS {
        p.push(c);
    }
    p
}

/// Read the version key out of an extracted `dev_flash` tree.
///
/// # Errors
///
/// [`FirmwareInstallError::VersionUnreadable`] when the file is absent
/// or unreadable, and [`FirmwareInstallError::VersionUnparseable`] when
/// it carries no version field.
pub(super) fn read_version(dev_flash_dir: &Path) -> Result<String, FirmwareInstallError> {
    let path = version_txt_path(dev_flash_dir);
    let text = std::fs::read_to_string(&path).map_err(|source| {
        FirmwareInstallError::VersionUnreadable {
            path: path.clone(),
            source,
        }
    })?;
    parse_version_txt(&text).ok_or_else(|| FirmwareInstallError::VersionUnparseable {
        path,
        leading: text
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(LEADING_SHOWN)
            .collect(),
    })
}

#[cfg(test)]
#[path = "tests/version_txt_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/version_txt_short_minor_tests.rs"]
mod short_minor_tests;
