//! PUP (PlayStation Update Package) container facts.
//!
//! Entry ids are a fixed table Sony assigns to the payloads a firmware
//! update carries; a reader locates a payload by id, never by position.
//!
//! The constants below are read from the 4.93 retail update package,
//! SHA-256 158471fd834f8ea8036136b6aab43cd86c7ba73d79ca30e0af3c0fe0001cf365:
//!
//! - It opens with the magic `SCEUF`, package version 1 and image
//!   version 0x10B94.
//! - The 0x30-byte header is followed by nine entry records, then
//!   nine hash records, each 0x20 bytes; the tables end at 0x270.
//! - The header's length field reads 0x290, which leaves 0x20 bytes
//!   after the tables that no CellGov reader interprets. Entry 0x100
//!   starts at 0x290.
//! - Its entry ids are 0x100, 0x101, 0x103, 0x200, 0x201, 0x202, 0x300,
//!   0x501 and 0x601.
//!
//! The reference console holds no update package and no disc, so this
//! file has no console-side copy.

/// Bytes of the fixed header at file offset 0, and the offset of the entry table.
pub const PUP_HEADER_SIZE: usize = 0x30;

/// Bytes of one entry-table record, and of one hash-table record.
pub const PUP_RECORD_SIZE: usize = 0x20;

/// `update_files.tar`: the TAR of SCE-wrapped dev_flash packages that
/// carries the firmware image itself.
///
/// Every retail update package carries this payload under this id. In
/// the 4.93 package it is 0xB97_3000 bytes, the largest entry.
pub const ENTRY_ID_UPDATE_FILES: u64 = 0x300;

/// `version.txt`: the firmware version the package carries, as the
/// one-line text a user sees (`4.93`).
///
/// It sits outside every SCE envelope, so a reader needs no key for it.
/// The 4.93 package's entry holds the five bytes `4.93\n`.
pub const ENTRY_ID_VERSION_TXT: u64 = 0x100;

/// The firmware version key a PUP's `version.txt` payload names.
///
/// The payload's first line is `<major>.<minor>` with a two-digit
/// minor. The key drops a leading zero on the major, so it spells the
/// version the way [`crate::format::dev_flash::parse_version_txt`] reads it out
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
