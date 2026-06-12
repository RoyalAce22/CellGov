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
    assert_eq!(
        resolve_ps3_vfs_root(&args),
        std::path::PathBuf::from("/custom/path")
    );
}

#[test]
fn resolve_ps3_vfs_root_default_is_project_relative() {
    let _guard = EnvGuard::unset("CELLGOV_PS3_VFS_ROOT");
    let args = sv(&["cli", "run-game", "--title", "flow"]);
    let got = resolve_ps3_vfs_root(&args);
    assert_eq!(got, std::path::PathBuf::from("tools/rpcs3/dev_hdd0"));
}
