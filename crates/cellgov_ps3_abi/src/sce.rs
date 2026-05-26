//! SCE / SELF container constants for PS3 firmware and signed
//! executables.
//!
//! Behaviour (the decrypter pipeline, the PUP unpacker) lives in
//! `cellgov_firmware::{sce,pup,crypto}`; this module is data only.
//!
//! Per-revision SELF APP keys plus the `SelfKey` struct and the
//! `app_key_for_revision` lookup helper currently live in
//! `cellgov_firmware::crypto`; they stay there because the APP_KEYS
//! table is tightly coupled with the const-fn helpers that construct
//! it. Move the standalone scalar keys here so dump-imports and the
//! decrypter share a single declaration.

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
/// Mirrors the value in RPCS3's `tools/rpcs3-src/rpcs3/Crypto/key_vault.cpp`.
pub const PUP_KEY: [u8; 0x40] = [
    CELLGOV-REDACTED-KEY
    0xED, 0xED, 0xBE, 0x6B, 0xE5, 0x13, 0x72, 0x4D, 0xD8, 0xF7, 0xB6, 0x91, 0xE8, 0x8A, 0x38, 0xF4,
    0xB5, 0x16, 0x2B, 0xFB, 0xEC, 0xBE, 0x3A, 0x62, 0x18, 0x5D, 0xD7, 0xC9, 0x4D, 0xA2, 0x22, 0x5A,
    0xDA, 0x3F, 0xBF, 0xCE, 0x55, 0x5B, 0x9E, 0xA9, 0x64, 0x98, 0x29, 0xEB, 0x30, 0xCE, 0x83, 0x66,
];

/// AES-256 encryption key for the outer SCE package envelope.
pub const SCEPKG_ERK: [u8; 0x20] = [
    CELLGOV-REDACTED-KEY
    0x56, 0x40, 0x93, 0x8D, 0x4D, 0xBC, 0xB2, 0xCB, 0x52, 0xC5, 0xA2, 0xF8, 0xB0, 0x2B, 0x10, 0x31,
];

/// AES-128 initialization vector for the outer SCE package envelope.
pub const SCEPKG_RIV: [u8; 0x10] = [
    CELLGOV-REDACTED-KEY,
];

/// `supplemental_header.type == 3` marks the NPDRM (NPD) header in
/// an NPDRM-wrapped SELF; presence selects the NPDRM decrypt prefix
/// over the APP-keyed one.
pub const SCE_SUPPLEMENTAL_KIND_NPDRM: u32 = 3;

/// AES-128 key applied (ECB) to the RAP-derived intermediate value to
/// produce the NPDRM layer key that decrypts the metadata-info
/// envelope. Mirrors `NP_KLIC_KEY` in RPCS3
/// `tools/rpcs3-src/rpcs3/Crypto/key_vault.h:107-109`.
pub const NP_KLIC_KEY: [u8; 0x10] = [
    CELLGOV-REDACTED-KEY,
];

/// Default klicensee for free-license (license == 3) NPDRM titles
/// when no RAP is supplied; RPCS3 substitutes this for the
/// `rap_to_rif` output. Mirrors `NP_KLIC_FREE` in
/// `tools/rpcs3-src/rpcs3/Crypto/key_vault.h:95-97`.
pub const NP_KLIC_FREE: [u8; 0x10] = [
    CELLGOV-REDACTED-KEY,
];

/// AES-128 key for the first ECB stage of `rap_to_rif`. The 16 RAP
/// bytes are ECB-decrypted with this key before the 5-round
/// PBOX/E1/E2 dance. Mirrors `RAP_KEY` in RPCS3
/// `tools/rpcs3-src/rpcs3/Crypto/key_vault.h:129-131`.
pub const RAP_KEY: [u8; 0x10] = [
    CELLGOV-REDACTED-KEY,
];

/// Byte-permutation indices applied per round of the
/// `rap_to_rif` post-ECB stage. Index `i` of the round output is
/// pulled from index `RAP_PBOX[i]` of the round input. Mirrors
/// `RAP_PBOX` in RPCS3
/// `tools/rpcs3-src/rpcs3/Crypto/key_vault.h:133-135`.
pub const RAP_PBOX: [u8; 0x10] = [
    CELLGOV-REDACTED-KEY,
];

/// First per-round substitution table consumed by the `rap_to_rif`
/// loop. Mirrors `RAP_E1` in RPCS3
/// `tools/rpcs3-src/rpcs3/Crypto/key_vault.h:137-139`.
pub const RAP_E1: [u8; 0x10] = [
    CELLGOV-REDACTED-KEY,
];

/// Second per-round substitution table consumed by the
/// `rap_to_rif` loop. Mirrors `RAP_E2` in RPCS3
/// `tools/rpcs3-src/rpcs3/Crypto/key_vault.h:141-143`.
pub const RAP_E2: [u8; 0x10] = [
    CELLGOV-REDACTED-KEY,
];
