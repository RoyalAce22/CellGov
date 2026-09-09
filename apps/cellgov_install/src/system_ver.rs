//! The firmware version key a title's `PS3_SYSTEM_VER` names.
//!
//! `PARAM.SFO` spells the floor `MM.mmmm` (`01.5000`); the store keys a
//! firmware entry by the version its own tree names (`1.50`). The two
//! spell one version, and this module holds the one translation.

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
    let major = major.strip_prefix('0').unwrap_or(major);
    Ok(format!("{major}.{}", &minor[..2]))
}

#[cfg(test)]
#[path = "tests/system_ver_tests.rs"]
mod tests;
