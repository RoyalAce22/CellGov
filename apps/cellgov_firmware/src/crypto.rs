//! AES keys and per-revision APP keys used to decrypt PS3 firmware and SELF files.

pub const PUP_KEY: [u8; 0x40] = [
    CELLGOV-REDACTED-KEY
    0xED, 0xED, 0xBE, 0x6B, 0xE5, 0x13, 0x72, 0x4D, 0xD8, 0xF7, 0xB6, 0x91, 0xE8, 0x8A, 0x38, 0xF4,
    0xB5, 0x16, 0x2B, 0xFB, 0xEC, 0xBE, 0x3A, 0x62, 0x18, 0x5D, 0xD7, 0xC9, 0x4D, 0xA2, 0x22, 0x5A,
    0xDA, 0x3F, 0xBF, 0xCE, 0x55, 0x5B, 0x9E, 0xA9, 0x64, 0x98, 0x29, 0xEB, 0x30, 0xCE, 0x83, 0x66,
];

pub const SCEPKG_ERK: [u8; 0x20] = [
    CELLGOV-REDACTED-KEY
    0x56, 0x40, 0x93, 0x8D, 0x4D, 0xBC, 0xB2, 0xCB, 0x52, 0xC5, 0xA2, 0xF8, 0xB0, 0x2B, 0x10, 0x31,
];

pub const SCEPKG_RIV: [u8; 0x10] = [
    CELLGOV-REDACTED-KEY,
];

pub struct SelfKey {
    pub erk: [u8; 0x20],
    pub riv: [u8; 0x10],
}

fn hex_to_bytes_32(s: &str) -> [u8; 0x20] {
    let mut out = [0u8; 0x20];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap();
    }
    out
}

fn hex_to_bytes_16(s: &str) -> [u8; 0x10] {
    let mut out = [0u8; 0x10];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap();
    }
    out
}

pub fn app_key_for_revision(revision: u16) -> Option<SelfKey> {
    let (erk_hex, riv_hex) = match revision {
        0x0000 => (
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
        0x0001 => (
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
        0x0002 => (
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
        0x0003 => (
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
        0x0004 => (
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
        0x0005 => (
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
        0x0006 => (
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
        0x0007 => (
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
        0x0008 => (
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
        0x0009 => (
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
        0x000A => (
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
        0x000B => (
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
        0x000C => (
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
        0x000D => (
            "CELLGOV-REDACTED-KEY",
            "CELLGOV-REDACTED-KEY",
        ),
        _ => return None,
    };
    Some(SelfKey {
        erk: hex_to_bytes_32(erk_hex),
        riv: hex_to_bytes_16(riv_hex),
    })
}
