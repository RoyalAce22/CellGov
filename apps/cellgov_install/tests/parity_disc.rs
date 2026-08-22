//! Parity: install a real decrypted PS3 disc image and assert the
//! extracted EBOOT matches what RPCS3 extracts from the same disc.
//!
//! The expected value is the committed digest in
//! `tests/fixtures/rpcs3_digests/digests.txt`. A retail disc image is
//! ~2 GB and operator-owned, so this suite is compiled only under the
//! `title-dumps` feature, which declares that image present: a missing
//! one is a hard failure rather than a skip.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap on unexpected failure is correct"
)]

use cellgov_install::game_install;
use std::path::PathBuf;

#[path = "common/digests.rs"]
mod digests;
#[path = "common/dumps.rs"]
mod dumps;
#[path = "common/scratch.rs"]
mod scratch;

/// Decrypted disc image for `content_id`, under the dump root.
fn disc_image(content_id: &str) -> PathBuf {
    dumps::title_dir(content_id).join(format!("{content_id}.iso"))
}

#[test]
fn decrypted_disc_install_matches_the_rpcs3_extracted_eboot() {
    const CONTENT_ID: &str = "BCES00664";
    let iso_path = disc_image(CONTENT_ID);
    assert!(
        iso_path.is_file(),
        "title-dumps: decrypted disc image {} not found. Point {} at \
         the root holding per-title dumps.",
        iso_path.display(),
        dumps::ENV_DUMPS_DIR,
    );

    let iso = std::fs::read(&iso_path).expect("read disc ISO");
    let scratch = scratch::ScratchDir::new("disc_parity");
    let vfs = scratch.join("vfs");

    // install_iso runs the APP-keyed decrypt-proof internally; success
    // means the disc was decrypted and the EBOOT loads.
    let outcome = game_install::install_iso(&iso, &iso, &vfs, &scratch.join("installs"), true)
        .expect("decrypted-disc install (incl. decrypt-proof) must succeed");
    assert_eq!(outcome.title_id, CONTENT_ID);

    let installed = vfs
        .join("dev_bdvd")
        .join(&outcome.title_id)
        .join("PS3_GAME/USRDIR/EBOOT.BIN");
    let references = digests::table();
    let want = references
        .get(&format!("eboot/{CONTENT_ID}"))
        .unwrap_or_else(|| panic!("digests.txt lists eboot/{CONTENT_ID}"));
    let got_bytes = std::fs::metadata(&installed)
        .unwrap_or_else(|e| panic!("stat {}: {e}", installed.display()))
        .len();
    // Length before digest: two hashes do not say by how much the
    // extracted file moved.
    assert_eq!(
        got_bytes, want.bytes,
        "CellGov extracted {got_bytes} bytes, the RPCS3 reference is {} bytes",
        want.bytes,
    );
    assert_eq!(
        digests::sha256_file(&installed),
        want.sha256,
        "CellGov-installed disc EBOOT must match the RPCS3-extracted \
         reference (see tests/fixtures/rpcs3_digests/README.md)"
    );
}
