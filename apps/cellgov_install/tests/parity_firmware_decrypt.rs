//! Bit-identical parity for the SELF decryption pipeline.
//!
//! For each module in [`MODULES`], decrypt `<name>.sprx` from the
//! CellGov firmware install and compare against the committed RPCS3
//! reference digest in `tests/fixtures/rpcs3_digests/digests.txt`, so
//! the only fixture a run needs is the encrypted firmware CellGov
//! installed itself.
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

#[path = "common/digests.rs"]
mod digests;

/// The mount `cellgov_install install` populates with firmware.
const FIRMWARE_EXTERNAL: &str = "vfs/dev_flash/sys/external";

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

/// The installed firmware directory, asserted present.
///
/// # Panics
///
/// If absent. `firmware-corpus` declares the install exists, so a
/// missing one is a failure rather than a skip.
fn firmware_external_dir() -> PathBuf {
    let dir = digests::workspace_root().join(FIRMWARE_EXTERNAL);
    assert!(
        dir.is_dir(),
        "firmware-corpus: no firmware at {}. Run \
         `cellgov_install install <PS3UPDAT.PUP>` to populate it.",
        dir.display()
    );
    dir
}

fn decrypt_and_compare(
    stem: &str,
    encrypted_dir: &Path,
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
    let references = digests::table();
    for stem in MODULES {
        decrypt_and_compare(stem, &encrypted_dir, &references);
    }
    eprintln!(
        "cellgov_install parity: compared {} module digests",
        MODULES.len()
    );
}
