//! Title / VFS-root / checkpoint resolution shared by run-game,
//! bench-boot, and bench-boot-once.

use super::args::find_flag_value;
use super::exit::die;
use crate::game;

const DEFAULT_TITLE_REGISTRY_DIR: &str = "docs/titles";

/// Resolve the active [`game::manifest::TitleManifest`] for a
/// subcommand, in priority order: `--title-manifest <path>`,
/// `--content-id <SERIAL>`, `--title <shortname>`.
///
/// # Errors
///
/// Any error in flag parsing or file loading prints a diagnostic
/// prefixed with `subcmd` and exits with status 1. A flag written
/// without a value hard-errors rather than falling through to the
/// next lookup.
pub(crate) fn resolve_title_manifest(
    args: &[String],
    subcmd: &str,
) -> game::manifest::TitleManifest {
    if let Some(p) = find_flag_value(args, "--title-manifest") {
        return game::manifest::TitleManifest::load_from_path(std::path::Path::new(&p))
            .unwrap_or_else(|e| die(&format!("{subcmd}: {e}")));
    }
    let registry =
        game::manifest::TitleRegistry::scan_dir(std::path::Path::new(DEFAULT_TITLE_REGISTRY_DIR))
            .unwrap_or_else(|e| die(&format!("{subcmd}: title registry: {e}")));
    if let Some(cid) = find_flag_value(args, "--content-id") {
        return registry.by_content_id(&cid).cloned().unwrap_or_else(|| {
            die(&format!(
                "{subcmd}: unknown content id '{cid}'. Known titles: {}",
                registry.known_names_csv()
            ))
        });
    }
    if let Some(sn) = find_flag_value(args, "--title") {
        return registry.by_short_name(&sn).cloned().unwrap_or_else(|| {
            die(&format!(
                "{subcmd}: unknown title '{sn}'. Known titles: {}",
                registry.known_names_csv()
            ))
        });
    }
    die(&format!(
        "{subcmd}: one of --title, --content-id, or --title-manifest is required. Known titles: {}",
        registry.known_names_csv()
    ));
}

pub(crate) fn resolve_checkpoint_override(
    args: &[String],
    subcmd: &str,
) -> Option<game::manifest::CheckpointTrigger> {
    match game::manifest::CheckpointTrigger::parse_from_args(args) {
        Some(Ok(cp)) => Some(cp),
        Some(Err(msg)) => die(&format!("{subcmd}: {msg}")),
        None => None,
    }
}

/// Resolve the PS3 VFS root using, in priority order: `--vfs-root
/// <path>`, `CELLGOV_PS3_VFS_ROOT` env var, then `tools/rpcs3/dev_hdd0`.
/// Existence is not verified here.
pub(crate) fn resolve_ps3_vfs_root(args: &[String]) -> std::path::PathBuf {
    if let Some(p) = find_flag_value(args, "--vfs-root") {
        return std::path::PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("CELLGOV_PS3_VFS_ROOT") {
        return std::path::PathBuf::from(p);
    }
    std::path::PathBuf::from("tools/rpcs3/dev_hdd0")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    /// RAII env-var scrubber: snapshots the current value, unsets it,
    /// and restores on drop (including during unwinding).
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
}
