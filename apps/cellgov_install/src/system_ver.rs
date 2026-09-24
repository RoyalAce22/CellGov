//! Sony system-software versions: their one ordering, and the firmware
//! version key a title's `PS3_SYSTEM_VER` names.
//!
//! `PARAM.SFO` spells the floor `MM.mmmm` (`01.5000`); the store keys a
//! firmware entry by the version its own tree names (`1.50`). The two
//! spell one version, and this module holds the one translation.

/// A Sony system-software version, ordered numerically.
///
/// `4.93`, the console's `version.txt` form and the store key, and
/// `04.9300`, the form a PARAM.SFO and the update metadata share, are
/// one version written two ways. The fraction is right-padded to four
/// digits, so both read as 4.9300, and a sub-revision such as `04.9312`
/// orders above them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SystemVersion {
    major: u32,
    minor: u32,
}

impl SystemVersion {
    /// Reads either spelling: a major that parses as a `u32`, a dot,
    /// and one to four fraction digits. `None` for a string no order can be read from.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let (major, minor) = value.split_once('.')?;
        if minor.is_empty() || minor.len() > 4 || !minor.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let mut padded = minor.to_string();
        while padded.len() < 4 {
            padded.push('0');
        }
        Some(Self {
            major: major.parse().ok()?,
            minor: padded.parse().ok()?,
        })
    }

    /// The store's firmware version key: the major without padding,
    /// and the first two fraction digits. A sub-revision names the
    /// entry of the version it revises.
    #[must_use]
    pub fn store_key(self) -> String {
        format!("{}.{:02}", self.major, self.minor / 100)
    }
}

/// Why a `PS3_SYSTEM_VER` value names no firmware version key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("PS3_SYSTEM_VER {value:?} is not of the form MM.mmmm (two digits, a dot, four digits)")]
pub struct SystemVerError {
    /// The value as the table spelled it.
    pub value: String,
}

/// The store's firmware version key for a `PS3_SYSTEM_VER` value.
///
/// The result is spelled the way the firmware's own `version.txt`
/// spells it: `01.5000` becomes `1.50`.
///
/// # Errors
///
/// [`SystemVerError`] unless `value` is two ASCII digits, a dot, and
/// four ASCII digits.
pub fn firmware_version_key(value: &str) -> Result<String, SystemVerError> {
    let refuse = || SystemVerError {
        value: value.to_string(),
    };
    let (major, minor) = value.split_once('.').ok_or_else(refuse)?;
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(major, 2) || !digits(minor, 4) {
        return Err(refuse());
    }
    SystemVersion::parse(value)
        .map(SystemVersion::store_key)
        .ok_or_else(refuse)
}

#[cfg(test)]
#[path = "tests/system_ver_tests.rs"]
mod tests;
