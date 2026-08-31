//! The firmware version key, read out of the extracted tree.
//!
//! `vsh/etc/version.txt` is what the console and RPCS3 both read, so
//! the key a store entry is named after is the same string a user sees
//! in the XMB: `release:04.9100:` becomes `4.91`.

use std::path::{Path, PathBuf};

use cellgov_ps3_abi::dev_flash::VERSION_TXT_COMPONENTS;

use super::error::FirmwareInstallError;

/// Digits the minor part keeps even when they are trailing zeros, so
/// `04.9000` reads as `4.90` rather than `4.9`.
const MINOR_DIGITS: usize = 2;

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
/// The file opens with a `<field>:<version>:` record whose version is
/// zero-padded to a fixed width (`04.9100`).
///
/// Over that fixed-width field -- the only shape a shipped
/// `version.txt` carries -- this is the string RPCS3 arrives at in
/// `utils::get_firmware_version` (`rpcs3/util/sysinfo.cpp`), so a tree
/// the two runners share resolves to one key. RPCS3 measures the kept
/// length from the start of the padded field rather than from the first
/// surviving digit, so the two agree only while exactly one leading
/// zero comes off; an unpadded `4.91` reads there as `4.9`. This port
/// also refuses field shapes RPCS3 would turn into an unusable
/// directory name, such as one carrying spaces or a second dot.
///
/// `None` unless the delimited field is `<digits>.<digits>`.
fn parse_version(text: &str) -> Option<String> {
    let start = text.find(':')? + 1;
    let rest = text.get(start..)?;
    let field = rest.get(..rest.find(':')?)?;

    let (major, minor) = field.split_once('.')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(major) || !digits(minor) {
        return None;
    }

    let major = major.trim_start_matches('0');
    let kept = minor
        .trim_end_matches('0')
        .len()
        .max(MINOR_DIGITS)
        .min(minor.len());
    Some(format!(
        "{}.{}",
        if major.is_empty() { "0" } else { major },
        &minor[..kept]
    ))
}

#[cfg(test)]
#[path = "tests/version_txt_tests.rs"]
mod tests;
