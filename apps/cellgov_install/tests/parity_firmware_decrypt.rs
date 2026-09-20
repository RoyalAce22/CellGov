//! Bit-identical parity for the SELF decryption pipeline.
//!
//! For each module in [`FIRMWARE_MODULES`], decrypt `<name>.sprx` from the
//! CellGov firmware install and compare against the committed RPCS3
//! reference digest in `tests/fixtures/rpcs3_digests/digests.txt`. The
//! table's `sprx/` rows name the firmware version and the input each
//! reference plaintext came from.
//!
//! Compiled only under `installed-firmware-tests`, which declares that install
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

use digests::FIRMWARE_MODULES;

/// The `sys/external` directory of the installed firmware `version`.
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
/// `installed-firmware-tests` declares the install exists, so each is a failure
/// rather than a skip.
fn firmware_external_dir(version: &str) -> PathBuf {
    let root = digests::workspace_root().join(DEFAULT_VFS_ROOT);
    let layout = StoreLayout::new(&root);
    let records = layout.installs_dir().join(ArtifactKind::Firmware.as_str());
    let entries = std::fs::read_dir(&records).unwrap_or_else(|e| {
        panic!(
            "installed-firmware-tests: no firmware install records at {}: {e}. Run \
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
                "installed-firmware-tests: reading an entry of {}: {e}",
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
                "installed-firmware-tests: two firmware install records claim version {}: {} and {}",
                record.artifact.version,
                prev.display(),
                path.display()
            );
        }
    }
    let Some(dir) = found.get(version) else {
        let installed: Vec<&str> = found.keys().map(String::as_str).collect();
        panic!(
            "installed-firmware-tests: the committed digests were captured from firmware \
             {version}, which is not installed under {} (installed: {})",
            root.display(),
            installed.join(", ")
        );
    };
    assert!(
        dir.is_dir(),
        "installed-firmware-tests: firmware is recorded but its tree at {} is missing",
        dir.display()
    );
    dir.clone()
}

/// How module `stem` of `firmware` fails to match its committed rows,
/// or `None` when its input and plaintext both match.
fn divergence(
    stem: &str,
    firmware: &str,
    encrypted_dir: &Path,
    keys: &KeyVault,
    references: &BTreeMap<String, digests::Reference>,
) -> Option<String> {
    let row = |key: String| {
        references.get(&key).unwrap_or_else(|| {
            panic!(
                "{stem}: no committed reference digest under key {key:?}; \
                 see tests/fixtures/rpcs3_digests/README.md"
            )
        })
    };
    let input = row(format!("sprx/{firmware}/{stem}"));
    let expected = row(format!("decrypted_masked/{stem}"));
    let sprx_path = encrypted_dir.join(format!("{stem}.sprx"));
    if !sprx_path.is_file() {
        return Some(format!(
            "{stem}: missing at {}. A PUP install writes every module in \
             FIRMWARE_MODULES; a gap means the install is partial or the \
             module set has been renamed.",
            sprx_path.display(),
        ));
    }
    let encrypted_bytes = std::fs::read(&sprx_path).unwrap();
    // A reference decrypted from another file says nothing about the
    // decrypt, so the input check runs first.
    let input_sha = digests::sha256_bytes(&encrypted_bytes);
    if encrypted_bytes.len() as u64 != input.bytes || input_sha != input.sha256 {
        return Some(format!(
            "{stem}: the installed {} ({input_sha}, {} bytes) is not the input the \
             reference plaintext was decrypted from ({}, {} bytes), so the decrypt \
             was not compared; see tests/fixtures/rpcs3_digests/README.md",
            sprx_path.display(),
            encrypted_bytes.len(),
            input.sha256,
            input.bytes,
        ));
    }
    let mut decrypted = match cellgov_install::sce::decrypt_self_to_elf(&encrypted_bytes, keys) {
        Ok(elf) => elf,
        Err(e) => return Some(format!("{stem}: decrypt failed: {e}")),
    };
    if decrypted.len() < 0x40 {
        return Some(format!(
            "{stem}: decrypt produced {} bytes, < ELF64 header",
            decrypted.len()
        ));
    }
    // Shape-check the SPRX inner ELF: this firmware set ships with
    // e_shoff = 0 and `decrypt_self_to_elf` copies it verbatim.
    for (field, range) in [
        ("e_shoff", 0x28..0x30),
        ("e_shnum", 0x3C..0x3E),
        ("e_shstrndx", 0x3E..0x40),
    ] {
        if decrypted[range].iter().any(|&b| b != 0) {
            return Some(format!(
                "{stem}: SPRX inner ELF unexpectedly carries non-zero {field}"
            ));
        }
    }
    cellgov_install::sce::mask_non_semantic_elf_bytes(&mut decrypted);
    // Length before digest: a size divergence is the common shape and
    // two hashes do not say by how much the output moved.
    if decrypted.len() as u64 != expected.bytes {
        return Some(format!(
            "{stem}: decrypt produced {} bytes, the committed reference is {} bytes",
            decrypted.len(),
            expected.bytes,
        ));
    }
    let got = digests::sha256_bytes(&decrypted);
    (got != expected.sha256).then(|| {
        format!(
            "{stem}: CellGov's decrypt of the reference input diverges from the \
             committed plaintext at equal length ({} bytes): {got}, reference {}. \
             Investigate the divergence rather than re-blessing; see \
             tests/fixtures/rpcs3_digests/README.md",
            expected.bytes, expected.sha256,
        )
    })
}

#[test]
fn firmware_prx_decrypt_matches_the_committed_rpcs3_reference() {
    let references = digests::table();
    let firmware = digests::reference_firmware(&references);
    let encrypted_dir = firmware_external_dir(&firmware);
    let keys = keys::vault();
    // The test compares every module before its verdict, so one
    // divergence does not hide the next.
    let failures: Vec<String> = FIRMWARE_MODULES
        .iter()
        .filter_map(|stem| divergence(stem, &firmware, &encrypted_dir, &keys, &references))
        .collect();
    eprintln!(
        "cellgov_install parity: compared {} modules of firmware {firmware}, {} diverged",
        FIRMWARE_MODULES.len(),
        failures.len(),
    );
    assert!(
        failures.is_empty(),
        "{} of {} firmware modules diverge:\n{}",
        failures.len(),
        FIRMWARE_MODULES.len(),
        failures.join("\n"),
    );
}
