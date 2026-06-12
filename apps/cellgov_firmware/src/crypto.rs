//! AES keys for PS3 firmware and SELF decryption.
//!
//! PUP / SCEPKG scalar keys live in [`cellgov_ps3_abi::sce`] and are
//! re-exported here for backward compatibility. APP keys mirror
//! RPCS3's `KeyVault::LoadSelfAPPKeys` table. Revisions 0x0012 and
//! 0x0015 have no entry in either source.

pub use cellgov_ps3_abi::sce::{PUP_KEY, SCEPKG_ERK, SCEPKG_RIV};

/// AES-256-CBC key + IV pair for one SELF revision's APP-key slot.
#[derive(Copy, Clone)]
pub struct SelfKey {
    /// 32-byte AES-256 key.
    pub erk: [u8; 0x20],
    /// 16-byte initialization vector.
    pub riv: [u8; 0x10],
}

const fn nibble(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => panic!("non-hex character in compile-time hex constant"),
    }
}

const fn hex32(s: &str) -> [u8; 0x20] {
    assert!(
        s.len() == 64,
        "ERK hex literal must be exactly 64 characters"
    );
    let bytes = s.as_bytes();
    let mut out = [0u8; 0x20];
    let mut i = 0;
    while i < 0x20 {
        out[i] = (nibble(bytes[i * 2]) << 4) | nibble(bytes[i * 2 + 1]);
        i += 1;
    }
    out
}

const fn hex16(s: &str) -> [u8; 0x10] {
    assert!(
        s.len() == 32,
        "RIV hex literal must be exactly 32 characters"
    );
    let bytes = s.as_bytes();
    let mut out = [0u8; 0x10];
    let mut i = 0;
    while i < 0x10 {
        out[i] = (nibble(bytes[i * 2]) << 4) | nibble(bytes[i * 2 + 1]);
        i += 1;
    }
    out
}

const fn key(erk_hex: &str, riv_hex: &str) -> SelfKey {
    SelfKey {
        erk: hex32(erk_hex),
        riv: hex16(riv_hex),
    }
}

/// APP-key table, sorted by revision. Gaps at 0x0012 and 0x0015.
const APP_KEYS: &[(u16, SelfKey)] = &[
    (
        0x0000,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0001,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0002,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0003,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0004,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0005,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0006,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0007,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0008,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0009,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x000A,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x000B,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x000C,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x000D,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x000E,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x000F,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0010,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0011,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0013,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0014,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0016,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0017,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0018,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0019,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x001A,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x001B,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x001C,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x001D,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
];

/// Look up the APP key for a SELF container's revision tag. Returns
/// `None` for the unknown-revision gaps (0x0012, 0x0015) and for any
/// revision past the highest known entry.
pub fn app_key_for_revision(revision: u16) -> Option<SelfKey> {
    APP_KEYS
        .iter()
        .find(|(rev, _)| *rev == revision)
        .map(|(_, k)| *k)
}

/// NPDRM SELF keys, selected by revision tag for NPDRM-wrapped
/// binaries. The APP table does not apply: each revision carries
/// distinct ERK/RIV for the two paths.
const NPDRM_KEYS: &[(u16, SelfKey)] = &[
    (
        0x0001,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0002,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0003,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0004,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0006,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0007,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0009,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x000A,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x000C,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x000D,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x000F,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0010,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0013,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0016,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x0019,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
    (
        0x001C,
        key(
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
    ),
];

/// Look up the NPDRM SELF key for a revision tag. Returns `None` for
/// revisions that have no NPDRM entry (notably 0x0000, 0x0005, 0x0008,
/// 0x000B, 0x000E, 0x0011, 0x0012, 0x0014, 0x0015, 0x0017, 0x0018,
/// 0x001A, 0x001B, and any revision past 0x001C).
pub fn npdrm_key_for_revision(revision: u16) -> Option<SelfKey> {
    NPDRM_KEYS
        .iter()
        .find(|(rev, _)| *rev == revision)
        .map(|(_, k)| *k)
}

#[cfg(test)]
#[path = "tests/crypto_tests.rs"]
mod tests;
