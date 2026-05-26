//! Bit-identical parity test for the SELF decryption pipeline.
//!
//! For each module in [`MODULES`], decrypt `<name>.sprx` and bit-
//! compare against the `<name>.prx` reference.
//!
//! # Configuration
//!
//! - `CELLGOV_FIRMWARE_DIR` (default: `firmware/sys/external`).
//! - `CELLGOV_DECRYPTED_REF_DIR` (default:
//!   `tools/rpcs3/dev_flash_decrypted/sys/external`).
//! - `CELLGOV_REQUIRE_PARITY_FIXTURES=1` makes missing fixtures fail
//!   instead of skip.

#![allow(
    clippy::print_stderr,
    reason = "integration test: stderr carries fixture-absent diagnostics"
)]
#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap on unexpected failure is correct"
)]

use std::path::{Path, PathBuf};

const ENV_FIRMWARE_DIR: &str = "CELLGOV_FIRMWARE_DIR";
const ENV_DECRYPTED_REF_DIR: &str = "CELLGOV_DECRYPTED_REF_DIR";
const ENV_REQUIRE_FIXTURES: &str = "CELLGOV_REQUIRE_PARITY_FIXTURES";

const DEFAULT_FIRMWARE_DIR: &str = "firmware/sys/external";
const DEFAULT_REF_DIR: &str = "tools/rpcs3/dev_flash_decrypted/sys/external";

/// Module stems present as both `<stem>.sprx` and `<stem>.prx`.
const MODULES: &[&str] = &[
    "libaudio",
    "libfs",
    "libgcm_sys",
    "libio",
    "liblv2",
    "libnet",
    "libnetctl",
    "libspurs_jq",
    "libsync2",
    "libsysmodule",
    "libsysutil",
    "libsysutil_np",
];

fn dir_from_env_or_default(env_key: &str, default: &str) -> PathBuf {
    std::env::var(env_key)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(default))
}

/// Returns `None` when either fixture dir is absent;
/// `CELLGOV_REQUIRE_PARITY_FIXTURES=1` promotes that to a panic.
fn locate_fixtures() -> Option<(PathBuf, PathBuf)> {
    let encrypted = dir_from_env_or_default(ENV_FIRMWARE_DIR, DEFAULT_FIRMWARE_DIR);
    let reference = dir_from_env_or_default(ENV_DECRYPTED_REF_DIR, DEFAULT_REF_DIR);
    if !encrypted.is_dir() || !reference.is_dir() {
        if std::env::var_os(ENV_REQUIRE_FIXTURES).is_some() {
            panic!(
                "{ENV_REQUIRE_FIXTURES} set but fixtures missing: \
                 encrypted={} (exists={}), reference={} (exists={})",
                encrypted.display(),
                encrypted.is_dir(),
                reference.display(),
                reference.is_dir(),
            );
        }
        eprintln!(
            "cellgov_firmware parity: skipping (encrypted={}, reference={}; \
             set {ENV_FIRMWARE_DIR} / {ENV_DECRYPTED_REF_DIR} or run \
             `cellgov_firmware install` to populate)",
            encrypted.display(),
            reference.display(),
        );
        return None;
    }
    Some((encrypted, reference))
}

fn decrypt_and_compare(stem: &str, encrypted_dir: &Path, reference_dir: &Path) {
    let sprx_path = encrypted_dir.join(format!("{stem}.sprx"));
    let prx_path = reference_dir.join(format!("{stem}.prx"));
    if !sprx_path.is_file() || !prx_path.is_file() {
        eprintln!(
            "cellgov_firmware parity ({stem}): skipping (sprx={} exists={}, prx={} exists={})",
            sprx_path.display(),
            sprx_path.is_file(),
            prx_path.display(),
            prx_path.is_file(),
        );
        return;
    }
    let encrypted_bytes = std::fs::read(&sprx_path).unwrap();
    let mut decrypted = cellgov_firmware::sce::decrypt_self_to_elf(&encrypted_bytes)
        .unwrap_or_else(|e| panic!("{stem}: decrypt failed: {e}"));
    let mut reference = std::fs::read(&prx_path).unwrap();
    // Asserted pre-mask so a dropped zeroing in decrypt_self_to_elf
    // cannot be hidden by re-masking both sides.
    assert_eq!(
        &decrypted[0x28..0x30],
        &[0u8; 8],
        "{stem}: e_shoff not zeroed by decrypt_self_to_elf"
    );
    assert_eq!(
        &decrypted[0x3C..0x3E],
        &[0u8; 2],
        "{stem}: e_shnum not zeroed by decrypt_self_to_elf"
    );
    assert_eq!(
        &decrypted[0x3E..0x40],
        &[0u8; 2],
        "{stem}: e_shstrndx not zeroed by decrypt_self_to_elf"
    );
    cellgov_firmware::sce::mask_non_semantic_elf_bytes(&mut decrypted);
    cellgov_firmware::sce::mask_non_semantic_elf_bytes(&mut reference);
    assert_eq!(
        decrypted.len(),
        reference.len(),
        "{stem}: decrypted length {} != reference length {}",
        decrypted.len(),
        reference.len(),
    );
    if decrypted != reference {
        let first_diff = decrypted
            .iter()
            .zip(reference.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        panic!(
            "{stem}: byte mismatch at offset 0x{first_diff:x} \
             (decrypted=0x{:02x}, reference=0x{:02x})",
            decrypted[first_diff], reference[first_diff],
        );
    }
}

#[test]
fn min_viable_prx_decrypt_matches_pre_decrypted_reference() {
    let Some((encrypted_dir, reference_dir)) = locate_fixtures() else {
        return;
    };
    for stem in MODULES {
        decrypt_and_compare(stem, &encrypted_dir, &reference_dir);
    }
}

// Game-title SELF byte-identity gates: flOw + SSHD against
// RPCS3-derived SHA-256 oracles; WipEout against a CellGov-derived
// refactor-invariance baseline.

use sha2::{Digest, Sha256};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// flOw (NPUA80001) RPCS3-decrypted EBOOT.elf SHA-256, unmasked.
const FLOW_ELF_SHA256_UNMASKED: [u8; 32] = [
    0x19, 0xf6, 0x11, 0xa3, 0x28, 0x8c, 0x08, 0x81, 0xd4, 0xf3, 0x8a, 0xf0, 0x13, 0xc2, 0x12, 0xf0,
    0x8f, 0x82, 0x82, 0x2a, 0xd6, 0x77, 0xca, 0x58, 0x4f, 0x48, 0xbd, 0x88, 0xe6, 0x75, 0xce, 0xb2,
];

/// flOw RPCS3-decrypted EBOOT.elf SHA-256, after applying
/// `mask_non_semantic_elf_bytes` (zeroes e_shoff/e_shnum/e_shstrndx).
const FLOW_ELF_SHA256_MASKED: [u8; 32] = [
    0xce, 0x97, 0x49, 0x54, 0x23, 0x4b, 0xfd, 0x8d, 0xa7, 0x0c, 0x9c, 0xc5, 0x0f, 0xb1, 0x47, 0x28,
    0xdb, 0x51, 0x33, 0x99, 0xa4, 0xc0, 0x16, 0x59, 0x44, 0xdd, 0x85, 0x99, 0x23, 0x9c, 0xd3, 0x60,
];

/// SSHD (NPUA80068) RPCS3-decrypted EBOOT.elf SHA-256, unmasked.
const SSHD_ELF_SHA256_UNMASKED: [u8; 32] = [
    0x8a, 0xe5, 0xc5, 0xd6, 0xdf, 0x35, 0xe4, 0x48, 0x93, 0xab, 0xa8, 0x86, 0x23, 0xa1, 0xac, 0x86,
    0x66, 0xf2, 0xda, 0xde, 0x11, 0x86, 0xbe, 0x4e, 0x44, 0x1c, 0x17, 0x0f, 0xc2, 0x4a, 0x5e, 0x19,
];

/// SSHD RPCS3-decrypted EBOOT.elf SHA-256, masked form.
const SSHD_ELF_SHA256_MASKED: [u8; 32] = [
    0xc2, 0xe5, 0x85, 0xc3, 0xfe, 0xdc, 0x76, 0xb4, 0xe2, 0x9f, 0x80, 0x26, 0x2f, 0x62, 0x5c, 0x8d,
    0xad, 0xf7, 0x30, 0x4e, 0x31, 0x2a, 0x9b, 0x7b, 0x90, 0x34, 0x51, 0xa9, 0x83, 0xb6, 0x99, 0x42,
];

/// WipEout (BCES00664) CellGov-decrypted EBOOT.BIN SHA-256, unmasked.
/// CellGov-derived refactor-invariance baseline, not an RPCS3 oracle.
const WIPEOUT_ELF_SHA256_UNMASKED: [u8; 32] = [
    0x46, 0xb1, 0x4e, 0xba, 0x7c, 0x85, 0x22, 0x29, 0x82, 0x19, 0xf7, 0x3a, 0x79, 0x91, 0xea, 0x5b,
    0x5e, 0x56, 0xe9, 0x9c, 0xef, 0x34, 0x8c, 0xa2, 0x14, 0xfb, 0x1f, 0x90, 0x1d, 0xb7, 0x63, 0x1a,
];

const FLOW_BIN_RELATIVE: &str = "tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.BIN";
const FLOW_RAP_RELATIVE: &str =
    "tools/rpcs3/dev_hdd0/home/00000001/exdata/UP9000-NPUA80001_00-FLOWPS3PROMOTION.rap";
const SSHD_BIN_RELATIVE: &str = "tools/rpcs3/dev_hdd0/game/NPUA80068/USRDIR/EBOOT.BIN";
const SSHD_RAP_RELATIVE: &str =
    "tools/rpcs3/dev_hdd0/home/00000001/exdata/UP9000-NPUA80068_00-STARDUSTFULL0001.rap";
const WIPEOUT_BIN_RELATIVE: &str = "tools/rpcs3/dev_bdvd/BCES00664/PS3_GAME/USRDIR/EBOOT.BIN";

/// Decrypt an NPDRM-wrapped EBOOT.BIN via CellGov + RAP-derived klic,
/// then assert against the unmasked and masked oracle hashes via the
/// C.2 decision tree. Skips silently if the operator-supplied
/// fixtures are absent from this checkout.
fn npdrm_byte_identity(
    title: &str,
    bin_rel: &str,
    rap_rel: &str,
    expected_unmasked: &[u8; 32],
    expected_masked: &[u8; 32],
) {
    let ws = workspace_root();
    let bin_path = ws.join(bin_rel);
    let rap_path = ws.join(rap_rel);
    if !bin_path.is_file() || !rap_path.is_file() {
        eprintln!(
            "cellgov_firmware C.2 ({title}): skipping; missing {} or {}",
            bin_path.display(),
            rap_path.display(),
        );
        return;
    }
    let bin = std::fs::read(&bin_path).unwrap();
    let rap = std::fs::read(&rap_path).unwrap();
    let rap_arr: [u8; 16] = rap.as_slice().try_into().expect("RAP must be 16 bytes");
    let klic = cellgov_firmware::npdrm::rap_to_klic(&rap_arr);
    let mut elf = cellgov_firmware::npdrm::decrypt_self_to_elf_npdrm(&bin, &klic)
        .unwrap_or_else(|e| panic!("{title}: NPDRM decrypt failed: {e}"));

    let got_unmasked: [u8; 32] = Sha256::digest(&elf).into();
    if &got_unmasked == expected_unmasked {
        eprintln!("{title}: byte-identical to RPCS3 oracle (unmasked)");
        return;
    }
    cellgov_firmware::sce::mask_non_semantic_elf_bytes(&mut elf);
    let got_masked: [u8; 32] = Sha256::digest(&elf).into();
    if &got_masked == expected_masked {
        eprintln!(
            "{title}: identical to RPCS3 oracle after masking \
             section-header fields (non-semantic divergence)"
        );
        return;
    }
    panic!(
        "{title}: CellGov decrypt diverges from RPCS3 oracle:\n  \
         got unmasked = {}\n  exp unmasked = {}\n  got masked   = {}\n  exp masked   = {}",
        hex_str(&got_unmasked),
        hex_str(expected_unmasked),
        hex_str(&got_masked),
        hex_str(expected_masked),
    );
}

fn hex_str(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn flow_eboot_byte_identity_against_rpcs3_oracle() {
    npdrm_byte_identity(
        "flOw",
        FLOW_BIN_RELATIVE,
        FLOW_RAP_RELATIVE,
        &FLOW_ELF_SHA256_UNMASKED,
        &FLOW_ELF_SHA256_MASKED,
    );
}

#[test]
fn sshd_eboot_byte_identity_against_rpcs3_oracle() {
    npdrm_byte_identity(
        "SSHD",
        SSHD_BIN_RELATIVE,
        SSHD_RAP_RELATIVE,
        &SSHD_ELF_SHA256_UNMASKED,
        &SSHD_ELF_SHA256_MASKED,
    );
}

#[test]
fn wipeout_eboot_refactor_invariance() {
    let ws = workspace_root();
    let bin_path = ws.join(WIPEOUT_BIN_RELATIVE);
    if !bin_path.is_file() {
        eprintln!(
            "cellgov_firmware C.2 (WipEout): skipping; missing {}",
            bin_path.display()
        );
        return;
    }
    let bin = std::fs::read(&bin_path).unwrap();
    let elf = cellgov_firmware::sce::decrypt_self_to_elf(&bin)
        .unwrap_or_else(|e| panic!("WipEout: APP decrypt failed: {e}"));
    let got: [u8; 32] = Sha256::digest(&elf).into();
    assert_eq!(
        got,
        WIPEOUT_ELF_SHA256_UNMASKED,
        "WipEout APP decrypt diverges from refactor-invariance \
         baseline: got {} != expected {}",
        hex_str(&got),
        hex_str(&WIPEOUT_ELF_SHA256_UNMASKED),
    );
}
