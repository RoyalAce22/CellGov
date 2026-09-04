//! PS3 VFS-root resolution precedence across CLI flag, env var, and default.

use super::*;

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
    let got = resolve_ps3_vfs_root_inner(Some(Path::new("/custom/path")), None)
        .expect("flag names a root");
    assert_eq!(got, PathBuf::from("/custom/path"));
}

#[test]
fn resolve_ps3_vfs_root_default_is_project_relative() {
    let _guard = EnvGuard::unset("CELLGOV_PS3_VFS_ROOT");
    let default_root = Path::new(cellgov_install::store::DEFAULT_VFS_ROOT);
    let got = resolve_ps3_vfs_root(None);
    assert_eq!(got, default_root.join("dev_hdd0"));
    assert_eq!(
        crate::cli::keys::fixed_vault_root(),
        Some(default_root),
        "the vault is read beside dev_hdd0, where the installers write it",
    );
}

#[test]
fn an_empty_vfs_root_flag_is_refused_rather_than_meaning_the_current_directory() {
    let err = resolve_ps3_vfs_root_inner(Some(Path::new("")), None)
        .expect_err("empty root names no directory");
    assert!(
        err.contains("--vfs-root"),
        "message names the origin: {err}"
    );
}

#[test]
fn an_empty_vfs_root_env_var_is_refused_rather_than_meaning_the_current_directory() {
    let err = resolve_ps3_vfs_root_inner(None, Some(std::ffi::OsString::new()))
        .expect_err("empty root names no directory");
    assert!(
        err.contains("CELLGOV_PS3_VFS_ROOT"),
        "message names the origin: {err}"
    );
}

#[test]
fn a_set_vfs_root_env_var_beats_the_default() {
    let got = resolve_ps3_vfs_root_inner(None, Some(std::ffi::OsString::from("/from/env")))
        .expect("non-empty env root resolves");
    assert_eq!(got, PathBuf::from("/from/env"));
}

#[test]
fn the_vfs_root_flag_beats_a_set_env_var() {
    let got = resolve_ps3_vfs_root_inner(
        Some(Path::new("/from/flag")),
        Some(std::ffi::OsString::from("/from/env")),
    )
    .expect("flag wins");
    assert_eq!(got, PathBuf::from("/from/flag"));
}
