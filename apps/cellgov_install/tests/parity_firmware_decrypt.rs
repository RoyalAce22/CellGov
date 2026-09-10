//! Bit-identical parity for the SELF decryption pipeline.
//!
//! For each module in [`MODULES`], decrypt `<name>.sprx` from the
//! CellGov firmware install and compare against the committed RPCS3
//! reference digest in `tests/fixtures/rpcs3_digests/digests.txt`.
//!
//! Compiled only under `firmware-corpus`, which declares that install
//! present: every module is asserted, never skipped.

#![allow(
    clippy::print_stderr,
    reason = "integration test: stderr carries the per-module comparison census"
)]
#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap on unexpected failure is correct"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_install::keys::KeyVault;
use cellgov_install::store::{ArtifactKind, InstallRecord, StoreLayout, DEFAULT_VFS_ROOT};
use cellgov_ps3_abi::format::dev_flash::FLASH_MOUNT;

#[path = "common/digests.rs"]
mod digests;
#[path = "common/keys.rs"]
mod keys;

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

/// The firmware version the committed digests were captured from.
const REFERENCE_FIRMWARE_VERSION: &str = "4.93";

/// The `sys/external` directory of [`REFERENCE_FIRMWARE_VERSION`].
///
/// The store keys firmware on version, so the install record names the
/// tree.
///
/// # Panics
///
/// Panics when the store:
///
/// - holds no firmware entry,
/// - holds no entry for that version,
/// - names one version twice, or
/// - names a tree that is gone.
///
/// `firmware-corpus` declares the install exists, so each is a failure
/// rather than a skip.
fn firmware_external_dir() -> PathBuf {
    let root = digests::workspace_root().join(DEFAULT_VFS_ROOT);
    let layout = StoreLayout::new(&root);
    let records = layout.installs_dir().join(ArtifactKind::Firmware.as_str());
    let entries = std::fs::read_dir(&records).unwrap_or_else(|e| {
        panic!(
            "firmware-corpus: no firmware install records at {}: {e}. Run \
             `cellgov firmware install <PS3UPDAT.PUP>` to populate the store.",
            records.display()
        )
    });
    let mut found: BTreeMap<String, PathBuf> = BTreeMap::new();
    for entry in entries {
        // Skipping an unreadable entry would drop a firmware from the
        // census.
        let entry = entry.unwrap_or_else(|e| {
            panic!(
                "firmware-corpus: reading an entry of {}: {e}",
                records.display()
            )
        });
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".install.toml"))
        {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        let record = InstallRecord::parse(&text)
            .unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()));
        let tree = layout
            .resolve_store_path(&record.artifact.store_path)
            .join(FLASH_MOUNT)
            .join("sys")
            .join("external");
        if let Some(prev) = found.insert(record.artifact.version.clone(), tree) {
            panic!(
                "firmware-corpus: two firmware install records claim version {}: {} and {}",
                record.artifact.version,
                prev.display(),
                path.display()
            );
        }
    }
    let Some(dir) = found.get(REFERENCE_FIRMWARE_VERSION) else {
        let installed: Vec<&str> = found.keys().map(String::as_str).collect();
        panic!(
            "firmware-corpus: the committed digests were captured from firmware \
             {REFERENCE_FIRMWARE_VERSION}, which is not installed under {} (installed: {})",
            root.display(),
            installed.join(", ")
        );
    };
    assert!(
        dir.is_dir(),
        "firmware-corpus: firmware is recorded but its tree at {} is missing",
        dir.display()
    );
    dir.clone()
}

fn decrypt_and_compare(
    stem: &str,
    encrypted_dir: &Path,
    keys: &KeyVault,
    references: &BTreeMap<String, digests::Reference>,
) {
    let sprx_path = encrypted_dir.join(format!("{stem}.sprx"));
    let key = format!("decrypted_masked/{stem}");
    let Some(expected) = references.get(&key) else {
        panic!(
            "{stem}: no committed reference digest under key {key:?}; \
             see tests/fixtures/rpcs3_digests/README.md"
        );
    };
    assert!(
        sprx_path.is_file(),
        "firmware-corpus: {stem} missing at {}. A PUP install writes \
         every module in MODULES; a gap means the install is partial \
         or the module set has been renamed.",
        sprx_path.display(),
    );
    let encrypted_bytes = std::fs::read(&sprx_path).unwrap();
    let mut decrypted = cellgov_install::sce::decrypt_self_to_elf(&encrypted_bytes, keys)
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
    // Length before digest: a size divergence is the common shape and
    // two hashes do not say by how much the output moved.
    assert_eq!(
        decrypted.len() as u64,
        expected.bytes,
        "{stem}: decrypt produced {} bytes, the RPCS3 reference is {} bytes",
        decrypted.len(),
        expected.bytes,
    );
    let got = digests::sha256_bytes(&decrypted);
    assert_eq!(
        got, expected.sha256,
        "{stem}: CellGov's decrypt diverges from the RPCS3 reference at \
         equal length ({} bytes). Investigate the divergence rather than \
         re-blessing; see tests/fixtures/rpcs3_digests/README.md",
        expected.bytes,
    );
}

#[test]
fn firmware_prx_decrypt_matches_the_committed_rpcs3_reference() {
    let encrypted_dir = firmware_external_dir();
    let keys = keys::vault();
    let references = digests::table();
    for stem in MODULES {
        decrypt_and_compare(stem, &encrypted_dir, &keys, &references);
    }
    eprintln!(
        "cellgov_install parity: compared {} module digests",
        MODULES.len()
    );
}
