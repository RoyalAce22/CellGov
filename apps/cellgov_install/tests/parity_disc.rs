//! Opt-in parity: install a real *decrypted* PS3 disc image and assert
//! the extracted EBOOT byte-matches the RPCS3-extracted reference under
//! `tools/rpcs3/dev_bdvd/`.
//!
//! Unlike the flOw PKG test, this one does NOT auto-discover: a retail
//! disc image is ~2GB, far too heavy to read on every `cargo test`. It
//! runs only when `CELLGOV_DISC_ISO` points at a decrypted image, and
//! skips cleanly otherwise (the `path.exists()` skip-gate pattern).

use cellgov_install::game_install;
use std::path::{Path, PathBuf};

/// Repo-root-relative RPCS3 disc reference for the given title-id.
fn reference_eboot(title_id: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../tools/rpcs3/dev_bdvd/{title_id}/PS3_GAME/USRDIR/EBOOT.BIN"
    ))
}

#[test]
fn decrypted_disc_install_matches_rpcs3_extracted_eboot() {
    // Opt-in only: the env var must name a decrypted ISO.
    let Ok(iso_path) = std::env::var("CELLGOV_DISC_ISO") else {
        return;
    };
    let iso_path = PathBuf::from(iso_path);
    if !iso_path.exists() {
        return;
    }

    let iso = std::fs::read(&iso_path).expect("read disc ISO");

    let scratch = std::env::temp_dir().join(format!("cellgov_disc_parity_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let vfs = scratch.join("vfs");
    let installs = scratch.join("installs");

    // install_iso runs the APP-keyed decrypt-proof internally; success
    // means the disc was decrypted and the EBOOT loads.
    let outcome = game_install::install_iso(&iso, &iso, &vfs, &installs, true)
        .expect("decrypted-disc install (incl. decrypt-proof) must succeed");

    let installed = vfs
        .join("dev_bdvd")
        .join(&outcome.title_id)
        .join("PS3_GAME/USRDIR/EBOOT.BIN");
    let got = std::fs::read(&installed).expect("read installed EBOOT");

    let reference = reference_eboot(&outcome.title_id);
    if reference.exists() {
        let want = std::fs::read(&reference).expect("read reference EBOOT");
        assert_eq!(
            got, want,
            "CellGov-installed disc EBOOT must byte-match the RPCS3-extracted one"
        );
    }

    let _ = std::fs::remove_dir_all(&scratch);
}
