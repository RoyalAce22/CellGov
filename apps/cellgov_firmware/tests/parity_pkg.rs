//! Presence-gated parity: install the operator's real flOw PKG with
//! its RAP and assert the extracted `EBOOT.BIN` is byte-identical to
//! the RPCS3-extracted reference the boot path already consumes. This
//! is the load-bearing proof that `install-game` is a faithful
//! RPCS3-equivalent installer.
//!
//! The dump directory is read from `CELLGOV_FLOW_PKG_DIR` (falling
//! back to the operator's default location); the test skips cleanly
//! when neither the dump nor the reference tree is present, reusing
//! the `path.exists()` skip-gate pattern from
//! `cellgov_ppu`'s `parse_real_liblv2`.

use cellgov_firmware::game_install;
use std::path::{Path, PathBuf};

/// Operator dump directory holding the flOw `.pkg` and `.rap`.
fn flow_dump_dir() -> PathBuf {
    std::env::var("CELLGOV_FLOW_PKG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"D:/Emulation/ROMs/PS3/flOw"))
}

/// The RPCS3-extracted reference EBOOT, resolved relative to the repo
/// root (two levels above this crate's manifest).
fn reference_eboot() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.BIN")
}

/// First file in `dir` whose extension matches `ext` (ASCII-insensitive).
fn first_with_ext(dir: &Path, ext: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case(ext))
        })
}

#[test]
fn flow_pkg_install_matches_rpcs3_extracted_eboot() {
    let dump = flow_dump_dir();
    let reference = reference_eboot();
    // Presence gate: skip cleanly when the dump or reference is absent.
    if !dump.exists() || !reference.exists() {
        return;
    }
    let (Some(pkg_path), Some(rap_path)) =
        (first_with_ext(&dump, "pkg"), first_with_ext(&dump, "rap"))
    else {
        return;
    };

    let pkg = std::fs::read(&pkg_path).expect("read flOw PKG");
    let rap = std::fs::read(&rap_path).expect("read flOw RAP");

    let scratch = std::env::temp_dir().join(format!("cellgov_flow_parity_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let vfs = scratch.join("vfs");
    let installs = scratch.join("installs");

    // install_pkg runs the decrypt-proof gate internally, so a
    // successful return already proves the installed tree loads.
    let outcome = game_install::install_pkg(&pkg, Some(&rap), &vfs, &installs, true)
        .expect("flOw install (incl. decrypt-proof) must succeed");
    assert_eq!(outcome.title_id, "NPUA80001");
    assert!(outcome.rap_installed, "license-1/2 title installs its RAP");

    let installed = vfs.join("dev_hdd0/game/NPUA80001/USRDIR/EBOOT.BIN");
    let got = std::fs::read(&installed).expect("read installed EBOOT");
    let want = std::fs::read(&reference).expect("read reference EBOOT");
    assert_eq!(
        got, want,
        "CellGov-installed EBOOT must byte-match the RPCS3-extracted one"
    );

    let _ = std::fs::remove_dir_all(&scratch);
}
