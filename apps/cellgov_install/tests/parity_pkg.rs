//! Parity: install the operator's real flOw PKG with its RAP and
//! assert the extracted `EBOOT.BIN` matches what RPCS3 extracts from
//! the same package. This is the proof that `install-game` is a
//! faithful RPCS3-equivalent installer.
//!
//! The expected value is the committed digest in
//! `tests/fixtures/rpcs3_digests/digests.txt`, not a live RPCS3
//! install tree. The PKG itself is operator-owned, so this suite is
//! compiled only under the `title-corpus` feature and hard-asserts the
//! dump is present rather than skipping.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap on unexpected failure is correct"
)]

use cellgov_install::game_install;
use std::path::{Path, PathBuf};

#[path = "common/digests.rs"]
mod digests;
#[path = "common/scratch.rs"]
mod scratch;

/// Operator dump directory holding the flOw `.pkg` and `.rap`.
///
/// Corpus location, not a machine path: `CELLGOV_TITLE_DUMPS` names
/// the root holding per-title dump directories.
fn flow_dump_dir() -> PathBuf {
    let root = std::env::var("CELLGOV_TITLE_DUMPS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| digests::workspace_root().join("dumps"));
    root.join("NPUA80001")
}

/// The one file in `dir` whose extension matches `ext` (ASCII-insensitive).
///
/// # Panics
///
/// If `dir` is unreadable, or holds no such file, or holds more than
/// one. `read_dir` yields in host-defined order, so picking the first
/// of several would make which package gets installed a property of
/// the filesystem; the ambiguity is named instead. A wrong pick would
/// otherwise surface as a digest mismatch blaming the install pipeline.
fn only_with_ext(dir: &Path, ext: &str) -> PathBuf {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read dir {}: {e}", dir.display()));
    let mut hits: Vec<PathBuf> = entries
        .map(|e| {
            e.unwrap_or_else(|e| panic!("read an entry of {}: {e}", dir.display()))
                .path()
        })
        .filter(|p| {
            p.extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case(ext))
        })
        .collect();
    hits.sort();
    match hits.len() {
        1 => hits.remove(0),
        0 => panic!("no .{ext} in {}", dir.display()),
        _ => panic!(
            "{} holds {} .{ext} files ({}); leave exactly the one this \
             parity gate's digest was blessed against",
            dir.display(),
            hits.len(),
            hits.iter()
                .map(|p| p.file_name().unwrap_or_default().to_string_lossy())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

#[test]
fn only_with_ext_returns_the_single_match_and_names_every_other_outcome() {
    let scratch = scratch::ScratchDir::new("only_with_ext");
    let dir = scratch.join("dump");
    std::fs::create_dir_all(&dir).unwrap();

    let refusal = |dir: &Path| {
        std::panic::catch_unwind(|| only_with_ext(dir, "pkg"))
            .err()
            .and_then(|e| e.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| panic!("only_with_ext accepted {}", dir.display()))
    };

    assert!(refusal(&dir).contains("no .pkg in"));

    std::fs::write(dir.join("flow.PKG"), b"x").unwrap();
    // Extension match is ASCII-insensitive, and a non-matching
    // neighbour is not a candidate.
    std::fs::write(dir.join("flow.rap"), b"x").unwrap();
    assert_eq!(only_with_ext(&dir, "pkg"), dir.join("flow.PKG"));

    std::fs::write(dir.join("other.pkg"), b"x").unwrap();
    let msg = refusal(&dir);
    assert!(msg.contains("holds 2 .pkg files"), "got {msg:?}");
    // Both candidates are named, so the report does not depend on
    // read_dir order.
    assert!(
        msg.contains("flow.PKG") && msg.contains("other.pkg"),
        "got {msg:?}"
    );

    assert!(refusal(&scratch.join("absent")).contains("read dir"));
}

#[test]
fn flow_pkg_install_matches_the_rpcs3_extracted_eboot() {
    let dump = flow_dump_dir();
    assert!(
        dump.is_dir(),
        "title-corpus: flOw dump directory {} not found. Point \
         CELLGOV_TITLE_DUMPS at the root holding per-title dumps.",
        dump.display()
    );
    let (pkg_path, rap_path) = (only_with_ext(&dump, "pkg"), only_with_ext(&dump, "rap"));

    let pkg = std::fs::read(&pkg_path).unwrap();
    let rap = std::fs::read(&rap_path).unwrap();

    let scratch = scratch::ScratchDir::new("flow_parity");
    let vfs = scratch.join("vfs");

    // install_pkg runs the decrypt-proof gate internally, so a
    // successful return already proves the installed tree loads.
    let outcome =
        game_install::install_pkg(&pkg, Some(&rap), &vfs, &scratch.join("installs"), true)
            .expect("flOw install (incl. decrypt-proof) must succeed");
    assert_eq!(outcome.title_id, "NPUA80001");
    assert!(outcome.rap_installed, "license-1/2 title installs its RAP");

    let installed = vfs.join("dev_hdd0/game/NPUA80001/USRDIR/EBOOT.BIN");
    let want = digests::table();
    let want = want
        .get("eboot/NPUA80001")
        .expect("digests.txt lists eboot/NPUA80001");
    assert_eq!(
        &digests::sha256_file(&installed),
        want,
        "CellGov-installed EBOOT must match the RPCS3-extracted reference \
         (see tests/fixtures/rpcs3_digests/README.md)"
    );
}
