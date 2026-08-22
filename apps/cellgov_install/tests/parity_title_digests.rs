//! Game-title SELF byte-identity gates.
//!
//! Expected hashes live in `tests/parity_digests.toml`, one row per
//! content id. NPDRM rows (flOw / SSHD) carry RPCS3-derived unmasked
//! and masked SHA-256 hashes; APP rows (WipEout) carry a
//! CellGov-derived refactor-invariance baseline (unmasked only).
//!
//! Compiled only under `title-corpus`, which declares the operator
//! owns title dumps -- though not necessarily every row pinned here.
//! A row whose title is not installed is passed over; a row whose
//! title IS installed but whose pinned EBOOT or RAP path resolves
//! nothing fails as drift; a run that compared no row at all fails as
//! vacuous.

#![allow(
    clippy::print_stderr,
    reason = "integration test: stderr carries the per-title comparison census"
)]
#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap on unexpected failure is correct"
)]

use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[path = "common/digests.rs"]
mod digests;

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

fn workspace_root() -> PathBuf {
    digests::workspace_root()
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
/// Its presence separates an uninstalled title (a pass-over) from a
/// derived EBOOT path that no longer matches where the installer
/// writes (drift).
fn title_dir_for(content_id: &str, key: &str) -> PathBuf {
    let ws = workspace_root();
    match key {
        "npdrm" => ws.join("vfs/dev_hdd0/game").join(content_id),
        "app" => ws.join("vfs/dev_bdvd").join(content_id),
        other => panic!("{content_id}: unknown key {other:?}"),
    }
}

/// Refuse a pass-over that only looks like an absent fixture.
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
    if !bin_path.is_file() {
        assert_not_installed(entry, &bin_path);
        eprintln!(
            "cellgov_install eboot parity ({title}): not installed; missing {}",
            bin_path.display(),
        );
        return false;
    }
    // The RAP is written into `exdata/` by the same install that wrote
    // the EBOOT (`cellgov_install::game_install::install_pkg`), so for
    // an installed title its absence is drift. Passing over it would
    // drop both NPDRM rows while the remaining APP row -- a
    // CellGov-derived baseline rather than an RPCS3 reference -- kept
    // the suite's anti-vacuity floor satisfied and the run green.
    assert!(
        rap_path.is_file(),
        "{title} is installed under {} but its pinned RAP {} resolved \
         nothing: reinstall the title with its RAP, or drop the row from \
         parity_digests.toml",
        title_dir_for(&entry.content_id, &entry.key).display(),
        rap_path.display(),
    );
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
        assert_not_installed(entry, &bin_path);
        eprintln!(
            "cellgov_install eboot parity ({title}): not installed; missing {}",
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
    // title-corpus declares the operator owns title dumps. Which rows
    // they own is theirs to decide, but owning none makes this gate a
    // no-op that still reports ok, so the floor is one.
    assert!(
        compared > 0,
        "title-corpus is on but none of the {} pinned titles is installed. \
         Install one with `cellgov_install install-game` / `install-iso`, \
         or build without the feature.",
        titles.len()
    );
    eprintln!(
        "cellgov_install eboot parity: compared {compared}/{} titles",
        titles.len()
    );
}
