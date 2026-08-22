//! Bit-identical parity test for the SELF decryption pipeline.
//!
//! For each module in [`MODULES`], decrypt `<name>.sprx` from the
//! CellGov firmware install and compare against the committed RPCS3
//! reference digest. The expected value is data
//! (`tests/fixtures/rpcs3_digests/digests.txt`), not a live RPCS3
//! install tree, so the only fixture a run needs is the encrypted
//! firmware CellGov installed itself.
//!
//! # Configuration
//!
//! - `CELLGOV_FIRMWARE_DIR` (default: `vfs/dev_flash/sys/external`).
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

#[path = "common/digests.rs"]
mod digests;

const ENV_FIRMWARE_DIR: &str = "CELLGOV_FIRMWARE_DIR";
const ENV_REQUIRE_FIXTURES: &str = "CELLGOV_REQUIRE_PARITY_FIXTURES";

const DEFAULT_FIRMWARE_DIR: &str = "vfs/dev_flash/sys/external";

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

/// Env override verbatim; the default anchors to the workspace root
/// because integration tests run with the crate directory as CWD.
fn dir_from_env_or_default(env_key: &str, default: &str) -> PathBuf {
    std::env::var(env_key)
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace_root().join(default))
}

/// `CELLGOV_REQUIRE_PARITY_FIXTURES` truthiness: `0`, `false`, and
/// empty mean OFF; anything else means ON.
fn require_fixtures() -> bool {
    match std::env::var_os(ENV_REQUIRE_FIXTURES) {
        None => false,
        Some(v) => {
            let v = v.to_string_lossy();
            !(v.is_empty() || v == "0" || v.eq_ignore_ascii_case("false"))
        }
    }
}

/// Returns `None` when the installed firmware is absent;
/// `CELLGOV_REQUIRE_PARITY_FIXTURES=1` promotes that to a panic.
///
/// Only the encrypted half is a fixture -- the expected plaintext is
/// a committed digest, not a directory.
fn locate_fixtures() -> Option<PathBuf> {
    let encrypted = dir_from_env_or_default(ENV_FIRMWARE_DIR, DEFAULT_FIRMWARE_DIR);
    if !encrypted.is_dir() {
        if require_fixtures() {
            panic!(
                "{ENV_REQUIRE_FIXTURES} set but the encrypted firmware is missing: {}",
                encrypted.display(),
            );
        }
        eprintln!(
            "cellgov_install parity: skipping (encrypted={} absent; \
             set {ENV_FIRMWARE_DIR} or run `cellgov_install install` \
             to populate)",
            encrypted.display(),
        );
        return None;
    }
    Some(encrypted)
}

/// Returns `true` when the module was actually compared.
///
/// The expected value is the committed masked digest, so only the encrypted
/// half is a fixture: RPCS3's plaintext was captured once rather than
/// read from an install tree at test time.
fn decrypt_and_compare(stem: &str, encrypted_dir: &Path, require: bool) -> bool {
    let sprx_path = encrypted_dir.join(format!("{stem}.sprx"));
    let expected = digests::table();
    let key = format!("decrypted_masked/{stem}");
    let Some(expected) = expected.get(&key) else {
        panic!(
            "{stem}: no committed reference digest under key {key:?}; \
             see tests/fixtures/rpcs3_digests/README.md"
        );
    };
    if !sprx_path.is_file() {
        if require {
            panic!(
                "{ENV_REQUIRE_FIXTURES} set but ({stem}) encrypted fixture missing: \
                 sprx={}",
                sprx_path.display(),
            );
        }
        eprintln!(
            "cellgov_install parity ({stem}): skipping (sprx={} absent)",
            sprx_path.display(),
        );
        return false;
    }
    let encrypted_bytes = std::fs::read(&sprx_path).unwrap();
    let mut decrypted = cellgov_install::sce::decrypt_self_to_elf(&encrypted_bytes)
        .unwrap_or_else(|e| panic!("{stem}: decrypt failed: {e}"));
    assert!(
        decrypted.len() >= 0x40,
        "{stem}: decrypt produced {} bytes, < ELF64 header",
        decrypted.len()
    );
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
    cellgov_install::sce::mask_non_semantic_elf_bytes(&mut decrypted);
    let got = digests::sha256_bytes(&decrypted);
    assert_eq!(
        &got,
        expected,
        "{stem}: CellGov's decrypt diverges from the RPCS3 reference \
         ({} bytes produced). Investigate the divergence rather than \
         re-blessing; see tests/fixtures/rpcs3_digests/README.md",
        decrypted.len(),
    );
    true
}

#[test]
fn firmware_prx_decrypt_matches_pre_decrypted_reference() {
    let Some(encrypted_dir) = locate_fixtures() else {
        return;
    };
    let require = require_fixtures();
    let mut compared = 0usize;
    for stem in MODULES {
        if decrypt_and_compare(stem, &encrypted_dir, require) {
            compared += 1;
        }
    }
    // Anti-vacuity floor: the firmware dir exists, so a run that
    // compared nothing (e.g. a renamed module set) must not pass as
    // if it verified the pipeline.
    assert!(
        compared > 0,
        "the firmware dir exists but none of the {} modules was present \
         to compare; the fixture layout has drifted",
        MODULES.len()
    );
    eprintln!(
        "cellgov_install parity: compared {compared}/{} module pairs",
        MODULES.len()
    );
}

// Game-title SELF byte-identity gates. Expected hashes live in
// `tests/parity_digests.toml`; one row per content_id. NPDRM rows
// (flOw / SSHD) carry RPCS3-derived unmasked + masked SHA-256
// hashes; APP rows (WipEout) carry a CellGov-derived
// refactor-invariance baseline (unmasked only).

use serde::Deserialize;
use sha2::{Digest, Sha256};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[derive(Deserialize)]
struct DigestManifest {
    title: Vec<TitleDigest>,
}

#[derive(Deserialize)]
struct TitleDigest {
    content_id: String,
    display: String,
    key: String,
    rap_filename: Option<String>,
    unmasked_sha256: String,
    masked_sha256: Option<String>,
}

fn load_title_digests() -> Vec<TitleDigest> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/parity_digests.toml");
    let s =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let parsed: DigestManifest =
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
            .join("vfs/dev_hdd0/game")
            .join(content_id)
            .join("USRDIR/EBOOT.BIN"),
        "app" => ws
            .join("vfs/dev_bdvd")
            .join(content_id)
            .join("PS3_GAME/USRDIR/EBOOT.BIN"),
        other => panic!("{content_id}: unknown key {other:?}"),
    }
}

/// The directory `install-game` / `install-iso` creates for a title.
///
/// Its presence separates "this title is not installed" (a documented
/// skip) from "the EBOOT path this test derives no longer matches
/// where the installer writes" (drift that must be loud).
fn title_dir_for(content_id: &str, key: &str) -> PathBuf {
    let ws = workspace_root();
    match key {
        "npdrm" => ws.join("vfs/dev_hdd0/game").join(content_id),
        "app" => ws.join("vfs/dev_bdvd").join(content_id),
        other => panic!("{content_id}: unknown key {other:?}"),
    }
}

/// Refuse a skip that only looks like an absent fixture.
fn assert_not_installed(entry: &TitleDigest, bin_path: &Path) {
    let title_dir = title_dir_for(&entry.content_id, &entry.key);
    assert!(
        bin_path.is_file() || !title_dir.is_dir(),
        "{} is installed under {} but the pinned EBOOT {} resolved \
         nothing: the derived path has drifted from where the \
         installer writes",
        entry.display,
        title_dir.display(),
        bin_path.display(),
    );
}

fn rap_path_for(rap_filename: &str) -> PathBuf {
    workspace_root()
        .join("vfs/dev_hdd0/home/00000001/exdata")
        .join(rap_filename)
}

/// NPDRM byte-identity gate; masked-identity is the contract.
///
/// Section-header layout (`e_shoff` / `e_shnum` / `e_shstrndx`) is
/// non-semantic, so the masked hash is the byte-identity check;
/// the unmasked hash is a strict-superset fast path that also
/// requires the section tables to coincide. See
/// [`cellgov_install::sce::mask_non_semantic_elf_bytes`] for the
/// section-vs-segment split.
///
/// Returns `true` when the title was actually compared.
fn run_npdrm_digest_check(entry: &TitleDigest) -> bool {
    let title = &entry.display;
    let bin_path = bin_path_for(&entry.content_id, &entry.key);
    let rap_filename = entry.rap_filename.as_ref().unwrap_or_else(|| {
        panic!("{title}: npdrm row requires rap_filename in parity_digests.toml")
    });
    let rap_path = rap_path_for(rap_filename);
    if !bin_path.is_file() || !rap_path.is_file() {
        if require_fixtures() {
            panic!(
                "{ENV_REQUIRE_FIXTURES} set but ({title}) fixture missing: \
                 bin={} exists={}, rap={} exists={}",
                bin_path.display(),
                bin_path.is_file(),
                rap_path.display(),
                rap_path.is_file(),
            );
        }
        assert_not_installed(entry, &bin_path);
        eprintln!(
            "cellgov_install C.2 ({title}): skipping; missing {} or {}",
            bin_path.display(),
            rap_path.display(),
        );
        return false;
    }
    let expected_unmasked = hex_to_bytes32(&entry.unmasked_sha256, &format!("{title} unmasked"));
    let expected_masked_hex = entry.masked_sha256.as_ref().unwrap_or_else(|| {
        panic!("{title}: npdrm row requires masked_sha256 in parity_digests.toml")
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
    let klic = cellgov_install::npdrm::rap_to_klic(&rap_arr);
    let mut elf = cellgov_install::npdrm::decrypt_self_to_elf_npdrm(&bin, &klic)
        .unwrap_or_else(|e| panic!("{title}: NPDRM decrypt failed: {e}"));
    assert!(
        elf.len() >= 0x40,
        "{title}: NPDRM decrypt produced {} bytes, < ELF64 header",
        elf.len()
    );

    let got_unmasked: [u8; 32] = Sha256::digest(&elf).into();
    if got_unmasked == expected_unmasked {
        eprintln!("{title}: byte-identical to the RPCS3 reference (unmasked)");
        return true;
    }
    cellgov_install::sce::mask_non_semantic_elf_bytes(&mut elf);
    let got_masked: [u8; 32] = Sha256::digest(&elf).into();
    if got_masked == expected_masked {
        eprintln!(
            "{title}: byte-identical to the RPCS3 reference (masked; \
             section-header layout is non-semantic)"
        );
        return true;
    }
    panic!(
        "{title}: CellGov decrypt diverges from the RPCS3 reference:\n  \
         got unmasked = {}\n  exp unmasked = {}\n  got masked   = {}\n  exp masked   = {}",
        hex_str(&got_unmasked),
        hex_str(&expected_unmasked),
        hex_str(&got_masked),
        hex_str(&expected_masked),
    );
}

/// Returns `true` when the title was actually compared.
fn run_app_digest_check(entry: &TitleDigest) -> bool {
    let title = &entry.display;
    let bin_path = bin_path_for(&entry.content_id, &entry.key);
    if !bin_path.is_file() {
        if require_fixtures() {
            panic!(
                "{ENV_REQUIRE_FIXTURES} set but ({title}) fixture missing: \
                 bin={}",
                bin_path.display(),
            );
        }
        assert_not_installed(entry, &bin_path);
        eprintln!(
            "cellgov_install C.2 ({title}): skipping; missing {}",
            bin_path.display()
        );
        return false;
    }
    let expected = hex_to_bytes32(&entry.unmasked_sha256, &format!("{title} unmasked"));
    let bin = std::fs::read(&bin_path).unwrap();
    let elf = cellgov_install::sce::decrypt_self_to_elf(&bin)
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
    true
}

fn hex_str(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn eboot_byte_identity_against_committed_digests() {
    let titles = load_title_digests();
    assert!(
        !titles.is_empty(),
        "parity_digests.toml must declare at least one [[title]] entry"
    );
    let mut compared = 0usize;
    for entry in &titles {
        let checked = match entry.key.as_str() {
            "npdrm" => run_npdrm_digest_check(entry),
            "app" => run_app_digest_check(entry),
            other => panic!(
                "{}: unknown key {:?} in parity_digests.toml",
                entry.content_id, other
            ),
        };
        if checked {
            compared += 1;
        }
    }
    // Anti-vacuity lives per title, in `assert_not_installed`: a title
    // whose install directory is present while the derived EBOOT path
    // resolves nothing fails there. Gating on `vfs/` instead would
    // fail a firmware-only VFS, which `cellgov_install install`
    // produces without writing any title.
    eprintln!(
        "cellgov_install C.2: compared {compared}/{} titles",
        titles.len()
    );
}
