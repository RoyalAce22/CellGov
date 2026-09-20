//! The operator's key vault, asserted present.
//!
//! The external-data suites decrypt real containers, so they need the real
//! key material: `CELLGOV_KEYS` naming a keys file or directory, or the
//! vault `cellgov keys import` wrote under the workspace's
//! `vfs/`. A missing vault fails the suite; nothing skips.

// Each integration test binary compiles this module separately and
// uses a different subset of it.
#![allow(dead_code)]

use std::path::PathBuf;

use cellgov_install::keys::{KeyVault, ENV_KEYS};

/// The workspace `vfs/`, where the installed content and an imported
/// vault live; the test binary's working directory is the crate, not
/// the workspace.
pub fn workspace_vfs() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.join("vfs")
}

/// Where the operator's vault is: `CELLGOV_KEYS` as set, else the one
/// imported under the workspace `vfs/`. For a suite that spawns a
/// binary, which reads its own vault relative to its own VFS root.
///
/// # Panics
///
/// If neither names one, or `CELLGOV_KEYS` is set but empty.
pub fn location() -> PathBuf {
    KeyVault::locate_from(std::env::var_os(ENV_KEYS), &workspace_vfs()).unwrap_or_else(|e| {
        panic!(
            "this suite decrypts real containers and needs the operator's key \
             vault: set {ENV_KEYS} to a keys file or directory, or run \
             `cellgov keys import <keys>` ({e})"
        )
    })
}

/// Load the operator's vault.
///
/// # Panics
///
/// If neither `CELLGOV_KEYS` nor an imported vault under the workspace
/// `vfs/` yields one, or the one found does not parse.
pub fn vault() -> KeyVault {
    KeyVault::load_for_vfs(&workspace_vfs()).unwrap_or_else(|e| {
        panic!(
            "this suite decrypts real containers and needs the operator's key \
             vault: set {ENV_KEYS} to a keys file or directory, or run \
             `cellgov keys import <keys>` ({e})"
        )
    })
}
