//! PS3 VFS-root resolution precedence across CLI flag, env var, and default.

use super::*;

fn sv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// RAII env-var scrubber: snapshots the current value, unsets it,
/// restores on drop.
struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    fn unset(key: &'static str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

#[test]
fn resolve_ps3_vfs_root_prefers_cli_flag() {
    let args = sv(&[
        "cli",
        "run-game",
        "--title",
        "flow",
        "--vfs-root",
        "/custom/path",
    ]);
    let got = resolve_ps3_vfs_root_inner(&args, None).expect("flag names a root");
    assert_eq!(got, std::path::PathBuf::from("/custom/path"));
}

#[test]
fn resolve_ps3_vfs_root_default_is_project_relative() {
    let _guard = EnvGuard::unset("CELLGOV_PS3_VFS_ROOT");
    let args = sv(&["cli", "run-game", "--title", "flow"]);
    let got = resolve_ps3_vfs_root(&args);
    assert_eq!(got, std::path::PathBuf::from("vfs/dev_hdd0"));
    assert_eq!(
        crate::cli::keys::fixed_vault_root(),
        Some(std::path::Path::new(
            cellgov_install::store::DEFAULT_VFS_ROOT
        )),
        "the vault is read beside dev_hdd0, where `cellgov_install` writes it",
    );
}

#[test]
fn an_empty_vfs_root_flag_is_refused_rather_than_meaning_the_current_directory() {
    let args = sv(&["cli", "run-game", "--title", "flow", "--vfs-root", ""]);
    let err = resolve_ps3_vfs_root_inner(&args, None).expect_err("empty root names no directory");
    assert!(
        err.contains("--vfs-root"),
        "message names the origin: {err}"
    );
}

#[test]
fn an_empty_vfs_root_env_var_is_refused_rather_than_meaning_the_current_directory() {
    let args = sv(&["cli", "run-game", "--title", "flow"]);
    let err = resolve_ps3_vfs_root_inner(&args, Some(std::ffi::OsString::new()))
        .expect_err("empty root names no directory");
    assert!(
        err.contains("CELLGOV_PS3_VFS_ROOT"),
        "message names the origin: {err}"
    );
}

#[test]
fn a_set_vfs_root_env_var_beats_the_default() {
    let args = sv(&["cli", "run-game", "--title", "flow"]);
    let got = resolve_ps3_vfs_root_inner(&args, Some(std::ffi::OsString::from("/from/env")))
        .expect("non-empty env root resolves");
    assert_eq!(got, std::path::PathBuf::from("/from/env"));
}

#[test]
fn the_vfs_root_flag_beats_a_set_env_var() {
    let args = sv(&["cli", "run-game", "--vfs-root", "/from/flag"]);
    let got = resolve_ps3_vfs_root_inner(&args, Some(std::ffi::OsString::from("/from/env")))
        .expect("flag wins");
    assert_eq!(got, std::path::PathBuf::from("/from/flag"));
}
