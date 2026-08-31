//! The firmware version key, read out of the extracted tree.
//!
//! `vsh/etc/version.txt` is the file the console reads, so the key a
//! store entry is named after is the same string a user sees in the
//! XMB: `release:04.9100:` becomes `4.91`.

use std::path::{Path, PathBuf};

use cellgov_ps3_abi::dev_flash::{
    VERSION_TXT_COMPONENTS, VERSION_TXT_MAJOR_DIGITS, VERSION_TXT_MINOR_DIGITS,
    VERSION_TXT_RELEASE_FIELD,
};

use super::error::FirmwareInstallError;

/// Minor digits the displayed version keeps. The rest are a
/// sub-revision, `00` in every firmware revision on hand.
const MINOR_DIGITS_SHOWN: usize = 2;

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
    parse_version(&text).ok_or(FirmwareInstallError::VersionUnparseable { path })
}

/// Extract the user-facing version from `version.txt`'s text.
///
/// The file opens with a `release:<version>:` record whose version is
/// fixed-width and zero-padded: two major digits, a dot, then four
/// minor digits.
///
/// `None` unless the leading record is `release` and its delimited
/// field carries exactly that shape.
fn parse_version(text: &str) -> Option<String> {
    let (record, rest) = text.split_once(':')?;
    if record != VERSION_TXT_RELEASE_FIELD {
        return None;
    }
    let field = rest.get(..rest.find(':')?)?;

    let (major, minor) = field.split_once('.')?;
    let fixed_width = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    if !fixed_width(major, VERSION_TXT_MAJOR_DIGITS)
        || !fixed_width(minor, VERSION_TXT_MINOR_DIGITS)
    {
        return None;
    }

    let major = major.trim_start_matches('0');
    Some(format!(
        "{}.{}",
        if major.is_empty() { "0" } else { major },
        &minor[..MINOR_DIGITS_SHOWN]
    ))
}

#[cfg(test)]
#[path = "tests/version_txt_tests.rs"]
mod tests;
