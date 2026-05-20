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
    // Pinned before the mask so dropping the zeroing in
    // decrypt_self_to_elf can't be papered over by re-masking both
    // sides on read.
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
