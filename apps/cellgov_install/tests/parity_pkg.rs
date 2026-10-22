//! Parity: install the operator's real flOw PKG with its RAP and
//! assert the extracted `EBOOT.BIN` matches what RPCS3 extracts from
//! the same package.
//!
//! The expected value is the committed digest in
//! `tests/fixtures/rpcs3_digests/digests.txt`. The PKG itself is
//! operator-owned, so this suite is compiled only under the
//! `title-dumps` feature, which declares that dump present: a missing
//! one is a hard failure rather than a skip.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap on unexpected failure is correct"
)]

use cellgov_install::game_install::{self, InstallOptions};
use std::path::PathBuf;

#[path = "common/digests.rs"]
mod digests;
#[path = "common/dump_pick.rs"]
mod dump_pick;
use dump_pick::only_with_ext;
#[path = "common/dumps.rs"]
mod dumps;
#[path = "common/keys.rs"]
mod keys;
#[path = "common/scratch.rs"]
mod scratch;

/// Operator dump directory holding the flOw `.pkg` and `.rap`.
fn flow_dump_dir() -> PathBuf {
    dumps::title_dir("NPUA80001")
}

#[test]
fn flow_pkg_install_matches_the_rpcs3_extracted_eboot() {
    let dump = flow_dump_dir();
    assert!(
        dump.is_dir(),
        "title-dumps: flOw dump directory {} not found. Point {} at \
         the root holding per-title dumps.",
        dump.display(),
        dumps::ENV_DUMPS_DIR,
    );
    let (pkg_path, rap_path) = (only_with_ext(&dump, "pkg"), only_with_ext(&dump, "rap"));

    let pkg = std::fs::read(&pkg_path).unwrap();
    let rap = std::fs::read(&rap_path).unwrap();

    let scratch = scratch::scratch_labeled("flow_parity");
    let vfs = scratch.join("vfs");

    // install_pkg runs the decrypt-proof gate internally, so a
    // successful return already proves the installed tree loads.
    let outcome = game_install::install_pkg(
        &pkg,
        Some(&rap),
        &keys::vault(),
        &vfs,
        InstallOptions {
            force: true,
            ..Default::default()
        },
    )
    .expect("flOw install (incl. decrypt-proof) must succeed");
    assert_eq!(outcome.title_id, "NPUA80001");
    assert!(outcome.rap_installed, "license-1/2 title installs its RAP");
    assert!(
        !outcome.rap_ignored,
        "a RAP this license consumes is not reported as dropped"
    );

    let installed = vfs.join("dev_hdd0/game/NPUA80001/USRDIR/EBOOT.BIN");
    let want = digests::table();
    let want = want
        .get("eboot/NPUA80001")
        .expect("digests.txt lists eboot/NPUA80001");
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
        "CellGov-installed EBOOT must match the RPCS3-extracted reference \
         (see tests/fixtures/rpcs3_digests/README.md)"
    );
}
