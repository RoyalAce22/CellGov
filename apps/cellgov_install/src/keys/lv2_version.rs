//! The firmware versions that label an LV2 keyset.
//!
//! An LV2 keyset opens the kernels of a run of firmware versions, so a
//! version range labels it. The kernel SELF's program identification
//! header carries the version, and the vault matches it against each
//! keyset's range.

use std::fmt;

use cellgov_ps3_abi::format::sce::{self_version, self_version_major, self_version_minor};

/// The firmware versions that label one LV2 keyset, both ends
/// included; a single version has `lo == hi`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Lv2Versions {
    /// The first version, as a SELF version word.
    pub lo: u64,
    /// The last version, as a SELF version word.
    pub hi: u64,
}

impl Lv2Versions {
    /// The one-version range.
    #[must_use]
    pub const fn single(version: u64) -> Self {
        Self {
            lo: version,
            hi: version,
        }
    }

    /// Whether `version` lies in the range.
    #[must_use]
    pub const fn contains(self, version: u64) -> bool {
        self.lo <= version && version <= self.hi
    }

    /// Parse a label. The spellings read:
    ///
    /// - `3.55`, one version;
    /// - `3.60-3.61`, a range, or `3.60~3.61` as a key table spells one;
    /// - `3-60-3-61`, a range as a name split into words spells it;
    /// - the 16 hex digits of one version word.
    ///
    /// A range whose ends are out of order gives `None`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if let Some(version) = parse_version_word(text) {
            return Some(Self::single(version));
        }
        let parts: Vec<&str> = text.split(['.', '-', '~']).collect();
        let (lo, hi) = match parts[..] {
            [major, minor] => {
                let version = dotted_version(major, minor)?;
                (version, version)
            }
            [lo_major, lo_minor, hi_major, hi_minor] => (
                dotted_version(lo_major, lo_minor)?,
                dotted_version(hi_major, hi_minor)?,
            ),
            _ => return None,
        };
        (lo <= hi).then_some(Self { lo, hi })
    }
}

/// `major.minor` as a version word: the major is one or two decimal
/// digits, the minor exactly two, kept as the BCD digits they spell.
fn dotted_version(major: &str, minor: &str) -> Option<u64> {
    fn decimal(part: &str, len: std::ops::RangeInclusive<usize>) -> Option<&str> {
        (len.contains(&part.len()) && part.bytes().all(|b| b.is_ascii_digit())).then_some(part)
    }
    let major: u16 = decimal(major, 1..=2)?.parse().ok()?;
    let minor_bcd = u16::from_str_radix(decimal(minor, 2..=2)?, 16).ok()?;
    Some(self_version(major, minor_bcd))
}

impl fmt::Display for Lv2Versions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.lo == self.hi {
            f.write_str(&version_label(self.lo))
        } else {
            write!(f, "{}-{}", version_label(self.lo), version_label(self.hi))
        }
    }
}

/// The 16 hex digits of a version word, as a scetool `version=` line
/// spells it.
fn parse_version_word(text: &str) -> Option<u64> {
    (text.len() == 16 && text.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| u64::from_str_radix(text, 16).ok())
        .flatten()
}

/// A version word as `3.55`, or as its 16 hex digits when the dotted
/// form would not read back through [`Lv2Versions::parse`]. The dotted
/// form needs a major of at most two decimal digits, a minor of two
/// BCD digits, and a zero low word.
#[must_use]
pub fn version_label(version: u64) -> String {
    let major = self_version_major(version);
    let minor = self_version_minor(version);
    let digits_only = version & 0xFFFF_FFFF == 0
        && major <= 99
        && minor <= 0xFF
        && (minor >> 4) <= 9
        && (minor & 0xF) <= 9;
    if digits_only {
        format!("{major}.{minor:02x}")
    } else {
        format!("{version:016x}")
    }
}

#[cfg(test)]
#[path = "tests/lv2_version_tests.rs"]
mod tests;
