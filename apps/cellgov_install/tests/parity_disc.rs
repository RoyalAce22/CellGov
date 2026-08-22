//! Parity: install a real decrypted PS3 disc image and assert the
//! extracted EBOOT matches what RPCS3 extracts from the same disc.
//!
//! The expected value is the committed digest in
//! `tests/fixtures/rpcs3_digests/digests.txt`, not a live RPCS3
//! install tree. A retail disc image is ~2 GB and operator-owned, so
//! this suite is compiled only under the `title-corpus` feature and
//! hard-asserts its input is present rather than skipping.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap on unexpected failure is correct"
)]

use cellgov_install::game_install;
use std::path::PathBuf;

#[path = "common/digests.rs"]
mod digests;
#[path = "common/scratch.rs"]
mod scratch;

/// Decrypted disc image for `content_id`, under the corpus dump root.
fn disc_image(content_id: &str) -> PathBuf {
    let root = std::env::var("CELLGOV_TITLE_DUMPS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| digests::workspace_root().join("dumps"));
    root.join(content_id).join(format!("{content_id}.iso"))
}

#[test]
fn decrypted_disc_install_matches_the_rpcs3_extracted_eboot() {
    const CONTENT_ID: &str = "BCES00664";
    let iso_path = disc_image(CONTENT_ID);
    assert!(
        iso_path.is_file(),
        "title-corpus: decrypted disc image {} not found. Point \
         CELLGOV_TITLE_DUMPS at the root holding per-title dumps.",
        iso_path.display()
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
    let digests = digests::table();
    let want = digests
        .get(&format!("eboot/{CONTENT_ID}"))
        .unwrap_or_else(|| panic!("digests.txt lists eboot/{CONTENT_ID}"));
    assert_eq!(
        &digests::sha256_file(&installed),
        want,
        "CellGov-installed disc EBOOT must match the RPCS3-extracted \
         reference (see tests/fixtures/rpcs3_digests/README.md)"
    );
}
