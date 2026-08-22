//! Where the local PS3 corpus lives on disk.
//!
//! Every path here is fixed and repo-relative because `cellgov_install`
//! produces them: `install` writes the firmware mounts under `vfs/`,
//! `install-game` and `install-iso` write the title mounts beside them.

#![allow(
    dead_code,
    reason = "each integration-test binary compiles this module separately and uses a subset"
)]

use std::path::PathBuf;

/// Workspace root, found by walking up to the manifest carrying
/// `[workspace]`. Integration tests run with the crate directory as
/// CWD, so relative corpus paths must anchor here.
pub fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if std::fs::read_to_string(p.join("Cargo.toml")).is_ok_and(|t| t.contains("[workspace]")) {
            return p;
        }
        assert!(
            p.pop(),
            "workspace root not found above {}",
            env!("CARGO_MANIFEST_DIR")
        );
    }
}

/// The `dev_flash` mount `cellgov_install install` populates.
pub fn dev_flash() -> PathBuf {
    workspace_root().join("vfs/dev_flash")
}

/// Firmware modules published to the guest.
///
/// # Panics
///
/// If the directory is absent. Callers have already declared the corpus
/// installed -- through the `firmware-corpus` feature or an `#[ignore]`
/// opt-in -- so absence is a failure rather than a reason to skip.
pub fn firmware_external_dir() -> PathBuf {
    let dir = dev_flash().join("sys/external");
    assert!(
        dir.is_dir(),
        "no PS3 firmware corpus at {}. Run `cellgov_install install <PS3UPDAT.PUP>` \
         to populate it.",
        dir.display()
    );
    dir
}
