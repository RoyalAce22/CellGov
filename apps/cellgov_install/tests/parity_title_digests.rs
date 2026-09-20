//! Game-title SELF byte-identity gates.
//!
//! Expected hashes live in `tests/parity_digests.toml`, one row per
//! content id. NPDRM rows (flOw / SSHD) carry RPCS3-derived unmasked
//! and masked SHA-256 hashes; APP rows (WipEout) carry a
//! CellGov-derived refactor-invariance baseline (unmasked only).
//!
//! Compiled only under `installed-title-tests`, which declares the operator
//! owns title dumps -- though not necessarily every row pinned here.
//! A row with no base record is passed over; a row whose record names
//! a tree with no EBOOT or RAP fails as drift; a run that compared no
//! row at all fails as vacuous.

#![allow(
    clippy::print_stderr,
    reason = "integration test: stderr carries the per-title comparison census"
)]
#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap on unexpected failure is correct"
)]

use std::path::PathBuf;

use cellgov_install::keys::KeyVault;
use cellgov_install::store::{
    Artifact, InstallRecord, StoreLayout, TitleId, TitleTree, DEFAULT_VFS_ROOT,
};
use sha2::{Digest, Sha256};

#[path = "common/digests.rs"]
mod digests;
#[path = "common/keys.rs"]
mod keys;
#[path = "common/title_digests.rs"]
mod title_digests;

use title_digests::{hex_to_bytes32, TitleDigest};

/// The `[title] distribution` tag that makes a base a disc tree.
const DISC_DISTRIBUTION: &str = "disc-iso";

/// Where a disc tree holds its executable, under the base directory.
const DISC_USRDIR: [&str; 2] = ["PS3_GAME", "USRDIR"];

/// Where an HDD game tree holds its executable.
const GAME_USRDIR: &str = "USRDIR";

fn workspace_root() -> PathBuf {
    digests::workspace_root()
}

fn layout() -> StoreLayout {
    StoreLayout::new(workspace_root().join(DEFAULT_VFS_ROOT))
}

/// One title's installed base, resolved through its install record.
struct InstalledBase {
    /// The tree the record's `store_path` names.
    dir: PathBuf,
    /// The executable inside that tree.
    eboot: PathBuf,
}

/// The base entry for `content_id`, or `None` when the store holds no
/// record for it.
///
/// The record names the tree, so this resolves the same title wherever
/// the installer puts it.
///
/// # Panics
///
/// Panics when the record:
///
/// - is unreadable,
/// - does not parse, or
/// - describes a kind that names no title.
fn installed_base(content_id: &str) -> Option<InstalledBase> {
    let layout = layout();
    let title_id = TitleId::new(content_id)
        .unwrap_or_else(|e| panic!("{content_id} is not a store title id: {e}"));
    let record_path = layout.record_path(&Artifact::TitleBase { title_id });
    let text = match std::fs::read_to_string(&record_path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => panic!("reading {}: {e}", record_path.display()),
    };
    let record = InstallRecord::parse(&text)
        .unwrap_or_else(|e| panic!("parsing {}: {e}", record_path.display()));
    let title = record.title.as_ref().unwrap_or_else(|| {
        panic!(
            "{} describes a {} entry, which names no title",
            record_path.display(),
            record.artifact.kind.as_str()
        )
    });
    let dir = layout.resolve_store_path(&record.artifact.store_path);
    let tree = if title.distribution == DISC_DISTRIBUTION {
        TitleTree::Disc
    } else {
        TitleTree::Game
    };
    let usrdir = match tree {
        TitleTree::Disc => DISC_USRDIR.iter().fold(dir.clone(), |d, p| d.join(p)),
        TitleTree::Game => dir.join(GAME_USRDIR),
    };
    Some(InstalledBase {
        eboot: usrdir.join("EBOOT.BIN"),
        dir,
    })
}

/// The EBOOT of an installed title, or `None` when it is not installed.
///
/// A record whose tree holds no EBOOT is drift: something removed the
/// executable the record names.
fn installed_eboot(entry: &TitleDigest) -> Option<PathBuf> {
    let base = installed_base(&entry.content_id)?;
    assert!(
        base.eboot.is_file(),
        "{} is installed under {} but its EBOOT {} resolved nothing: the tree \
         has drifted from the record that names it",
        entry.display,
        base.dir.display(),
        base.eboot.display(),
    );
    Some(base.eboot)
}

fn rap_path_for(rap_filename: &str) -> PathBuf {
    layout().live_exdata_dir().join(rap_filename)
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
fn run_npdrm_digest_check(entry: &TitleDigest, keys: &KeyVault) -> bool {
    let title = &entry.display;
    let rap_filename = entry.rap_filename.as_ref().unwrap_or_else(|| {
        panic!("{title}: npdrm row requires rap_filename in parity_digests.toml")
    });
    let rap_path = rap_path_for(rap_filename);
    let Some(bin_path) = installed_eboot(entry) else {
        eprintln!(
            "cellgov_install eboot parity ({title}): not installed; the store holds no \
             base record for {}",
            entry.content_id,
        );
        return false;
    };
    // The RAP is written into `exdata/` by the same install that wrote
    // the EBOOT (`cellgov_install::game_install::install_pkg`), so for
    // an installed title its absence is drift. Passing over it would
    // drop both NPDRM rows while the remaining APP row -- a
    // CellGov-derived baseline rather than an RPCS3 reference -- kept
    // the suite's anti-vacuity floor satisfied and the run green.
    assert!(
        rap_path.is_file(),
        "{title} is installed but its pinned RAP {} resolved nothing: reinstall \
         the title with its RAP, or drop the row from parity_digests.toml",
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
    let klic = cellgov_install::npdrm::rap_to_klic(keys, &rap_arr)
        .unwrap_or_else(|e| panic!("{title}: klicensee derivation failed: {e}"));
    let mut elf = cellgov_install::npdrm::decrypt_self_to_elf_npdrm(&bin, keys, &klic)
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
fn run_app_digest_check(entry: &TitleDigest, keys: &KeyVault) -> bool {
    let title = &entry.display;
    let Some(bin_path) = installed_eboot(entry) else {
        eprintln!(
            "cellgov_install eboot parity ({title}): not installed; the store holds no \
             base record for {}",
            entry.content_id,
        );
        return false;
    };
    let expected = hex_to_bytes32(&entry.unmasked_sha256, &format!("{title} unmasked"));
    let bin = std::fs::read(&bin_path).unwrap();
    let elf = cellgov_install::sce::decrypt_self_to_elf(&bin, keys)
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
    let titles = title_digests::load();
    title_digests::assert_well_formed(&titles);
    let keys = keys::vault();
    let mut compared = 0usize;
    let mut against_rpcs3 = 0usize;
    for entry in &titles {
        let checked = match entry.key.as_str() {
            "npdrm" => run_npdrm_digest_check(entry, &keys),
            "app" => run_app_digest_check(entry, &keys),
            other => panic!(
                "{}: unknown key {:?} in parity_digests.toml",
                entry.content_id, other
            ),
        };
        if checked {
            compared += 1;
            if entry.is_rpcs3_reference() {
                against_rpcs3 += 1;
            }
        }
    }
    // installed-title-tests declares the operator owns title dumps. Which rows
    // they own is theirs to decide, but owning none makes this gate a
    // no-op that still reports ok, so the floor is one.
    assert!(
        compared > 0,
        "installed-title-tests is on but none of the {} pinned titles is installed. \
         Install one with `cellgov title install`, or build without \
         the feature.",
        titles.len()
    );
    // Only npdrm rows carry an RPCS3-derived hash. A run of app rows
    // alone still gates -- against CellGov's own refactor-invariance
    // baseline -- but it is not cross-runner parity, and the census is
    // the only place a reader learns which of the two happened.
    eprintln!(
        "cellgov_install eboot parity: compared {compared}/{} titles \
         ({against_rpcs3} against the RPCS3 reference, {} against a \
         CellGov baseline)",
        titles.len(),
        compared - against_rpcs3,
    );
    if against_rpcs3 == 0 {
        eprintln!(
            "cellgov_install eboot parity: no installed title carries an \
             RPCS3 reference hash; this run held nothing against RPCS3. \
             Install an npdrm title to close that gap."
        );
    }
}
