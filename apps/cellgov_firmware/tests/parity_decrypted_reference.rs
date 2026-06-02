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

fn require_fixtures() -> bool {
    std::env::var_os(ENV_REQUIRE_FIXTURES).is_some()
}

/// Returns `None` when either fixture dir is absent;
/// `CELLGOV_REQUIRE_PARITY_FIXTURES=1` promotes that to a panic.
fn locate_fixtures() -> Option<(PathBuf, PathBuf)> {
    let encrypted = dir_from_env_or_default(ENV_FIRMWARE_DIR, DEFAULT_FIRMWARE_DIR);
    let reference = dir_from_env_or_default(ENV_DECRYPTED_REF_DIR, DEFAULT_REF_DIR);
    if !encrypted.is_dir() || !reference.is_dir() {
        if require_fixtures() {
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

fn decrypt_and_compare(stem: &str, encrypted_dir: &Path, reference_dir: &Path, require: bool) {
    let sprx_path = encrypted_dir.join(format!("{stem}.sprx"));
    let prx_path = reference_dir.join(format!("{stem}.prx"));
    if !sprx_path.is_file() || !prx_path.is_file() {
        if require {
            panic!(
                "{ENV_REQUIRE_FIXTURES} set but ({stem}) fixture half missing: \
                 sprx={} exists={}, prx={} exists={}",
                sprx_path.display(),
                sprx_path.is_file(),
                prx_path.display(),
                prx_path.is_file(),
            );
        }
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
    assert!(
        decrypted.len() >= 0x40,
        "{stem}: decrypt produced {} bytes, < ELF64 header",
        decrypted.len()
    );
    let mut reference = std::fs::read(&prx_path).unwrap();
    // Shape-check the SPRX inner ELF: this corpus ships with
    // e_shoff = 0 and `decrypt_self_to_elf` copies it verbatim.
    assert_eq!(
        &decrypted[0x28..0x30],
        &[0u8; 8],
        "{stem}: SPRX inner ELF unexpectedly carries non-zero e_shoff"
    );
    assert_eq!(
        &decrypted[0x3C..0x3E],
        &[0u8; 2],
        "{stem}: SPRX inner ELF unexpectedly carries non-zero e_shnum"
    );
    assert_eq!(
        &decrypted[0x3E..0x40],
        &[0u8; 2],
        "{stem}: SPRX inner ELF unexpectedly carries non-zero e_shstrndx"
    );
    cellgov_firmware::sce::mask_non_semantic_elf_bytes(&mut decrypted);
    cellgov_firmware::sce::mask_non_semantic_elf_bytes(&mut reference);
    assert_eq!(
        decrypted.len(),
        reference.len(),
        "{stem}: decrypt-length mismatch (decrypt produced {} bytes, reference is {} bytes)",
        decrypted.len(),
        reference.len(),
    );
    if decrypted != reference {
        let first_diff = decrypted
            .iter()
            .zip(reference.iter())
            .position(|(a, b)| a != b)
            .expect("buffers differ but no differing byte found");
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
    let require = require_fixtures();
    for stem in MODULES {
        decrypt_and_compare(stem, &encrypted_dir, &reference_dir, require);
    }
}

// Game-title SELF byte-identity gates. Oracles live in
// `tests/parity_oracles.toml`; one row per content_id. NPDRM rows
// (flOw / SSHD) carry RPCS3-derived unmasked + masked SHA-256
// hashes; APP rows (WipEout) carry a CellGov-derived
// refactor-invariance baseline (unmasked only).

use serde::Deserialize;
use sha2::{Digest, Sha256};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[derive(Deserialize)]
struct OracleManifest {
    title: Vec<Oracle>,
}

#[derive(Deserialize)]
struct Oracle {
    content_id: String,
    display: String,
    key: String,
    rap_filename: Option<String>,
    unmasked_sha256: String,
    masked_sha256: Option<String>,
}

fn load_oracles() -> Vec<Oracle> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/parity_oracles.toml");
    let s =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let parsed: OracleManifest =
        toml::from_str(&s).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
    parsed.title
}

fn hex_to_bytes32(s: &str, ctx: &str) -> [u8; 32] {
    assert_eq!(s.len(), 64, "{ctx}: hex must be 64 chars, got {}", s.len());
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
            .unwrap_or_else(|_| panic!("{ctx}: invalid hex byte at index {i} in {s:?}"));
    }
    out
}

fn bin_path_for(content_id: &str, key: &str) -> PathBuf {
    let ws = workspace_root();
    match key {
        "npdrm" => ws
            .join("tools/rpcs3/dev_hdd0/game")
            .join(content_id)
            .join("USRDIR/EBOOT.BIN"),
        "app" => ws
            .join("tools/rpcs3/dev_bdvd")
            .join(content_id)
            .join("PS3_GAME/USRDIR/EBOOT.BIN"),
        other => panic!("{content_id}: unknown key {other:?}"),
    }
}

fn rap_path_for(rap_filename: &str) -> PathBuf {
    workspace_root()
        .join("tools/rpcs3/dev_hdd0/home/00000001/exdata")
        .join(rap_filename)
}

/// NPDRM byte-identity gate; masked-identity is the contract.
///
/// Section-header layout (`e_shoff` / `e_shnum` / `e_shstrndx`) is
/// non-semantic, so the masked hash is the byte-identity check;
/// the unmasked hash is a strict-superset fast path that also
/// requires the section tables to coincide. See
/// [`cellgov_firmware::sce::mask_non_semantic_elf_bytes`] for the
/// section-vs-segment split.
fn run_npdrm_oracle(oracle: &Oracle) {
    let title = &oracle.display;
    let bin_path = bin_path_for(&oracle.content_id, &oracle.key);
    let rap_filename = oracle.rap_filename.as_ref().unwrap_or_else(|| {
        panic!("{title}: npdrm oracle requires rap_filename in parity_oracles.toml")
    });
    let rap_path = rap_path_for(rap_filename);
    if !bin_path.is_file() || !rap_path.is_file() {
        eprintln!(
            "cellgov_firmware C.2 ({title}): skipping; missing {} or {}",
            bin_path.display(),
            rap_path.display(),
        );
        return;
    }
    let expected_unmasked = hex_to_bytes32(&oracle.unmasked_sha256, &format!("{title} unmasked"));
    let expected_masked_hex = oracle.masked_sha256.as_ref().unwrap_or_else(|| {
        panic!("{title}: npdrm oracle requires masked_sha256 in parity_oracles.toml")
    });
    let expected_masked = hex_to_bytes32(expected_masked_hex, &format!("{title} masked"));

    let bin = std::fs::read(&bin_path).unwrap();
    let rap = std::fs::read(&rap_path).unwrap();
    let rap_arr: [u8; 16] = rap.as_slice().try_into().unwrap_or_else(|_| {
        panic!(
            "{title}: RAP {} is {} bytes, expected 16",
            rap_path.display(),
            rap.len()
        )
    });
    let klic = cellgov_firmware::npdrm::rap_to_klic(&rap_arr);
    let mut elf = cellgov_firmware::npdrm::decrypt_self_to_elf_npdrm(&bin, &klic)
        .unwrap_or_else(|e| panic!("{title}: NPDRM decrypt failed: {e}"));
    assert!(
        elf.len() >= 0x40,
        "{title}: NPDRM decrypt produced {} bytes, < ELF64 header",
        elf.len()
    );

    let got_unmasked: [u8; 32] = Sha256::digest(&elf).into();
    if got_unmasked == expected_unmasked {
        eprintln!("{title}: byte-identical to RPCS3 oracle (unmasked)");
        return;
    }
    cellgov_firmware::sce::mask_non_semantic_elf_bytes(&mut elf);
    let got_masked: [u8; 32] = Sha256::digest(&elf).into();
    if got_masked == expected_masked {
        eprintln!(
            "{title}: byte-identical to RPCS3 oracle (masked; \
             section-header layout is non-semantic)"
        );
        return;
    }
    panic!(
        "{title}: CellGov decrypt diverges from RPCS3 oracle:\n  \
         got unmasked = {}\n  exp unmasked = {}\n  got masked   = {}\n  exp masked   = {}",
        hex_str(&got_unmasked),
        hex_str(&expected_unmasked),
        hex_str(&got_masked),
        hex_str(&expected_masked),
    );
}

fn run_app_oracle(oracle: &Oracle) {
    let title = &oracle.display;
    let bin_path = bin_path_for(&oracle.content_id, &oracle.key);
    if !bin_path.is_file() {
        eprintln!(
            "cellgov_firmware C.2 ({title}): skipping; missing {}",
            bin_path.display()
        );
        return;
    }
    let expected = hex_to_bytes32(&oracle.unmasked_sha256, &format!("{title} unmasked"));
    let bin = std::fs::read(&bin_path).unwrap();
    let elf = cellgov_firmware::sce::decrypt_self_to_elf(&bin)
        .unwrap_or_else(|e| panic!("{title}: APP decrypt failed: {e}"));
    assert!(
        elf.len() >= 0x40,
        "{title}: APP decrypt produced {} bytes, < ELF64 header",
        elf.len()
    );
    let got: [u8; 32] = Sha256::digest(&elf).into();
    assert_eq!(
        got,
        expected,
        "{title} APP decrypt diverges from refactor-invariance \
         baseline: got {} != expected {}",
        hex_str(&got),
        hex_str(&expected),
    );
}

fn hex_str(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn eboot_byte_identity_against_oracles() {
    let oracles = load_oracles();
    assert!(
        !oracles.is_empty(),
        "parity_oracles.toml must declare at least one [[title]] entry"
    );
    for oracle in &oracles {
        match oracle.key.as_str() {
            "npdrm" => run_npdrm_oracle(oracle),
            "app" => run_app_oracle(oracle),
            other => panic!(
                "{}: unknown key {:?} in parity_oracles.toml",
                oracle.content_id, other
            ),
        }
    }
}
