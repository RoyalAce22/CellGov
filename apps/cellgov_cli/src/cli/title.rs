//! Title / VFS-root / checkpoint resolution shared by run-game,
//! bench-boot, and bench-boot-once.

use super::args::find_flag_value;
use super::exit::die;
use crate::game;

/// Registry directory every title-driven subcommand resolves
/// `--title` / `--content-id` against, relative to the working
/// directory.
pub(crate) const DEFAULT_TITLE_REGISTRY_DIR: &str = "docs/title_manifests";

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
/// <path>`, `CELLGOV_PS3_VFS_ROOT` env var, then `vfs/dev_hdd0` (the
/// CellGov-owned VFS that `cellgov_install install-game` / `install-iso`
/// populate). An empty value from either override is refused rather
/// than resolved against the current directory. Existence is not
/// verified here.
pub(crate) fn resolve_ps3_vfs_root(args: &[String]) -> std::path::PathBuf {
    resolve_ps3_vfs_root_inner(args, std::env::var_os("CELLGOV_PS3_VFS_ROOT"))
        .unwrap_or_else(|msg| die(&msg))
}

/// # Errors
///
/// An empty root from either override: it names no directory, so
/// every mount joined onto it would resolve against the process's
/// current directory. The manifest loader refuses an empty
/// `[source] path` for the same reason.
fn resolve_ps3_vfs_root_inner(
    args: &[String],
    env: Option<std::ffi::OsString>,
) -> Result<std::path::PathBuf, String> {
    let empty = |origin: &str| {
        format!(
            "{origin} is empty; an empty VFS root names no directory and would leave \
             dev_hdd0 / dev_bdvd / exdata to be resolved against the process's current \
             directory. Name the root, or unset it to take the vfs/dev_hdd0 default."
        )
    };
    if let Some(p) = find_flag_value(args, "--vfs-root") {
        if p.is_empty() {
            return Err(empty("--vfs-root"));
        }
        return Ok(std::path::PathBuf::from(p));
    }
    // A root path the platform accepts but that is not UTF-8 must
    // still reach the resolver.
    if let Some(p) = env {
        if p.is_empty() {
            return Err(empty("CELLGOV_PS3_VFS_ROOT"));
        }
        return Ok(std::path::PathBuf::from(p));
    }
    Ok(std::path::PathBuf::from("vfs/dev_hdd0"))
}

#[cfg(test)]
#[path = "tests/title_tests.rs"]
mod tests;
