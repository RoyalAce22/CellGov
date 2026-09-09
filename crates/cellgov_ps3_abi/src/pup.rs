//! PUP (PlayStation Update Package) container facts.
//!
//! Entry ids are a fixed table Sony assigns to the payloads a firmware
//! update carries; a reader locates a payload by id, never by position.

/// `update_files.tar`: the TAR of SCE-wrapped dev_flash packages that
/// carries the firmware image itself.
///
/// Every retail update package carries this payload under this id.
pub const ENTRY_ID_UPDATE_FILES: u64 = 0x300;

/// `version.txt`: the firmware version the package carries, as the
/// one-line text a user sees (`4.93`).
///
/// It sits outside every SCE envelope, so a reader needs no key for it.
pub const ENTRY_ID_VERSION_TXT: u64 = 0x100;

/// The firmware version key a PUP's `version.txt` payload names.
///
/// The payload's first line is `<major>.<minor>` with a two-digit
/// minor. The key drops a leading zero on the major, so it spells the
/// version the way [`crate::dev_flash::parse_version_txt`] reads it out
/// of the installed tree. `None` unless the first line has that shape.
pub fn parse_pup_version_txt(text: &str) -> Option<String> {
    let line = text.lines().next()?.trim();
    let (major, minor) = line.split_once('.')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(major) || minor.len() != 2 || !digits(minor) {
        return None;
    }
    let major = major.trim_start_matches('0');
    Some(format!(
        "{}.{minor}",
        if major.is_empty() { "0" } else { major }
    ))
}

#[cfg(test)]
#[path = "tests/pup_tests.rs"]
mod tests;
