//! SCE / SELF container constants for PS3 firmware and signed
//! executables.
//!
//! Behaviour (the decrypter pipeline, the PUP unpacker) lives in
//! `cellgov_firmware::{sce,pup,crypto}`; this module is data only.
//! Per-revision SELF APP keys and the `app_key_for_revision` lookup
//! live in `cellgov_firmware::crypto` alongside the const-fn
//! constructors that build the APP_KEYS table.

/// SCE container magic bytes (`"SCE\0"`) at offset 0 of every
/// signed PS3 file.
pub const SCE_MAGIC: [u8; 4] = *b"SCE\0";

/// `section_kind` value for SCE sections that describe the original
/// program-header table; consumed by the SELF decrypter to rebuild
/// the plaintext ELF's PHDR.
pub const SCE_SECTION_KIND_PHDR: u32 = 2;

/// `encryption_kind = 1`: section payload is stored plaintext (no
/// AES-128-CTR wrapper).
pub const SCE_ENC_KIND_PLAIN: u32 = 1;

/// `encryption_kind = 3`: section payload is wrapped in AES-128-CTR
/// using the section's nested key + IV.
pub const SCE_ENC_KIND_AES128_CTR: u32 = 3;

/// `compression_kind = 1`: section payload is stored uncompressed.
pub const SCE_COMP_KIND_NONE: u32 = 1;

/// `compression_kind = 2`: section payload is zlib-compressed and
/// the consumer must inflate before parsing.
pub const SCE_COMP_KIND_ZLIB: u32 = 2;

/// AES-256 key for PUP package payloads (PS3 firmware update files).
/// Mirrors the value in RPCS3's `key_vault.cpp` table.
pub const PUP_KEY: [u8; 0x40] = [
    0xF4, 0x91, 0xAD, 0x94, 0xC6, 0x81, 0x10, 0x96, 0x91, 0x5F, 0xD5, 0xD2, 0x44, 0x81, 0xAE, 0xDC,
    0xED, 0xED, 0xBE, 0x6B, 0xE5, 0x13, 0x72, 0x4D, 0xD8, 0xF7, 0xB6, 0x91, 0xE8, 0x8A, 0x38, 0xF4,
    0xB5, 0x16, 0x2B, 0xFB, 0xEC, 0xBE, 0x3A, 0x62, 0x18, 0x5D, 0xD7, 0xC9, 0x4D, 0xA2, 0x22, 0x5A,
    0xDA, 0x3F, 0xBF, 0xCE, 0x55, 0x5B, 0x9E, 0xA9, 0x64, 0x98, 0x29, 0xEB, 0x30, 0xCE, 0x83, 0x66,
];

/// AES-256 encryption key for the outer SCE package envelope.
pub const SCEPKG_ERK: [u8; 0x20] = [
    0xA9, 0x78, 0x18, 0xBD, 0x19, 0x3A, 0x67, 0xA1, 0x6F, 0xE8, 0x3A, 0x85, 0x5E, 0x1B, 0xE9, 0xFB,
    0x56, 0x40, 0x93, 0x8D, 0x4D, 0xBC, 0xB2, 0xCB, 0x52, 0xC5, 0xA2, 0xF8, 0xB0, 0x2B, 0x10, 0x31,
];

/// AES-128 initialization vector for the outer SCE package envelope.
pub const SCEPKG_RIV: [u8; 0x10] = [
    0x4A, 0xCE, 0xF0, 0x12, 0x24, 0xFB, 0xEE, 0xDF, 0x82, 0x45, 0xF8, 0xFF, 0x10, 0x21, 0x1E, 0x6E,
];

/// `supplemental_header.type == 3` marks the NPDRM (NPD) header in
/// an NPDRM-wrapped SELF; presence selects the NPDRM decrypt prefix
/// over the APP-keyed one.
pub const SCE_SUPPLEMENTAL_KIND_NPDRM: u32 = 3;

/// AES-128 key applied (ECB) to the RAP-derived intermediate value to
/// produce the NPDRM layer key that decrypts the metadata-info
/// envelope. Mirrors `NP_KLIC_KEY` in RPCS3's `key_vault.h`.
pub const NP_KLIC_KEY: [u8; 0x10] = [
    0xF2, 0xFB, 0xCA, 0x7A, 0x75, 0xB0, 0x4E, 0xDC, 0x13, 0x90, 0x63, 0x8C, 0xCD, 0xFD, 0xD1, 0xEE,
];

/// Default klicensee for free-license (license == 3) NPDRM titles
/// when no RAP is supplied; RPCS3 substitutes this for the
/// `rap_to_rif` output. Mirrors `NP_KLIC_FREE` in RPCS3's
/// `key_vault.h`.
pub const NP_KLIC_FREE: [u8; 0x10] = [
    0x72, 0xF9, 0x90, 0x78, 0x8F, 0x9C, 0xFF, 0x74, 0x57, 0x25, 0xF0, 0x8E, 0x4C, 0x12, 0x83, 0x87,
];

/// AES-128 key for the first ECB stage of `rap_to_rif`. The 16 RAP
/// bytes are ECB-decrypted with this key before the 5-round
/// PBOX/E1/E2 dance. Mirrors `RAP_KEY` in RPCS3's `key_vault.h`.
pub const RAP_KEY: [u8; 0x10] = [
    0x86, 0x9F, 0x77, 0x45, 0xC1, 0x3F, 0xD8, 0x90, 0xCC, 0xF2, 0x91, 0x88, 0xE3, 0xCC, 0x3E, 0xDF,
];

/// Byte-permutation indices applied per round of the
/// `rap_to_rif` post-ECB stage. Index `i` of the round output is
/// pulled from index `RAP_PBOX[i]` of the round input. Mirrors
/// `RAP_PBOX` in RPCS3's `key_vault.h`.
pub const RAP_PBOX: [u8; 0x10] = [
    0x0C, 0x03, 0x06, 0x04, 0x01, 0x0B, 0x0F, 0x08, 0x02, 0x07, 0x00, 0x05, 0x0A, 0x0E, 0x0D, 0x09,
];

/// First per-round substitution table consumed by the `rap_to_rif`
/// loop. Mirrors `RAP_E1` in RPCS3's `key_vault.h`.
pub const RAP_E1: [u8; 0x10] = [
    0xA9, 0x3E, 0x1F, 0xD6, 0x7C, 0x55, 0xA3, 0x29, 0xB7, 0x5F, 0xDD, 0xA6, 0x2A, 0x95, 0xC7, 0xA5,
];

/// Second per-round substitution table consumed by the
/// `rap_to_rif` loop. Mirrors `RAP_E2` in RPCS3's `key_vault.h`.
pub const RAP_E2: [u8; 0x10] = [
    0x67, 0xD4, 0x5D, 0xA3, 0x29, 0x6D, 0x00, 0x6A, 0x4E, 0x7C, 0x53, 0x7B, 0xF5, 0x53, 0x8C, 0x74,
];
