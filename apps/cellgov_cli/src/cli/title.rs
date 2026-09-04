//! Title and VFS-root resolution shared by the boot family and the
//! dev commands that compose the same guest tree.

use std::path::{Path, PathBuf};

use super::exit::die;
use super::parse::TitleSelector;
use crate::game;

/// Registry directory every title-driven subcommand resolves
/// `--title` / `--content-id` against, relative to the working
/// directory.
///
/// The store's `vfs/titles/` is a separate directory: it holds the
/// installed content, and this one holds the manifests that select it.
pub(crate) const DEFAULT_TITLE_REGISTRY_DIR: &str = "title_manifests";

/// The per-user license directory under a PS3 VFS root, where an
/// installed RAP lives.
pub(crate) fn exdata_dir(vfs_root: &Path) -> PathBuf {
    vfs_root.join("home").join(HDD0_USER).join("exdata")
}

/// The single modelled user profile on the internal HDD.
const HDD0_USER: &str = "00000001";

/// Resolve the active [`game::manifest::TitleManifest`] for a
/// subcommand, in priority order: `--title-manifest <path>`,
/// `--content-id <SERIAL>`, `--title <shortname>`.
///
/// # Errors
///
/// Any error in file loading or registry lookup prints a diagnostic
/// prefixed with `subcmd` and exits with status 1.
pub(crate) fn resolve_title_manifest(
    selector: &TitleSelector,
    subcmd: &str,
) -> game::manifest::TitleManifest {
    if let Some(p) = &selector.title_manifest {
        return game::manifest::TitleManifest::load_from_path(p)
            .unwrap_or_else(|e| die(&format!("{subcmd}: {e}")));
    }
    let registry = game::manifest::TitleRegistry::scan_dir(Path::new(DEFAULT_TITLE_REGISTRY_DIR))
        .unwrap_or_else(|e| die(&format!("{subcmd}: title registry: {e}")));
    if let Some(cid) = &selector.content_id {
        return registry.by_content_id(cid).cloned().unwrap_or_else(|| {
            die(&format!(
                "{subcmd}: unknown content id '{cid}'. Known titles: {}",
                registry.known_names_csv()
            ))
        });
    }
    if let Some(sn) = &selector.title {
        return registry.by_short_name(sn).cloned().unwrap_or_else(|| {
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

/// Resolve the PS3 VFS root using, in priority order: the `--vfs-root`
/// value, `CELLGOV_PS3_VFS_ROOT`, then `vfs/dev_hdd0` (the CellGov-owned
/// VFS the installers populate). An empty value from either override is
/// refused rather than resolved against the current directory. Existence
/// is not verified here.
///
/// Also fixes the root the operator's key vault is read under
/// ([`super::keys::fix_vault_root`]), so a subcommand that names a
/// relocated VFS decrypts under that VFS's imported vault. Resolve the
/// root before opening any guest image: the vault loads once, on the
/// first SCE-wrapped one.
pub(crate) fn resolve_ps3_vfs_root(flag: Option<&Path>) -> PathBuf {
    let root = resolve_ps3_vfs_root_inner(flag, std::env::var_os("CELLGOV_PS3_VFS_ROOT"))
        .unwrap_or_else(|msg| die(&msg));
    super::keys::fix_vault_root(&root);
    root
}

/// # Errors
///
/// An empty root from either override: it names no directory, so
/// every mount joined onto it would resolve against the process's
/// current directory. The manifest loader refuses an empty
/// `[source] path` for the same reason.
fn resolve_ps3_vfs_root_inner(
    flag: Option<&Path>,
    env: Option<std::ffi::OsString>,
) -> Result<PathBuf, String> {
    let empty = |origin: &str| {
        format!(
            "{origin} is empty; an empty VFS root names no directory and would leave \
             dev_hdd0 / dev_bdvd / exdata to be resolved against the process's current \
             directory. Name the root, or unset it to take the vfs/dev_hdd0 default."
        )
    };
    if let Some(p) = flag {
        if p.as_os_str().is_empty() {
            return Err(empty("--vfs-root"));
        }
        return Ok(p.to_path_buf());
    }
    // A root path the platform accepts but that is not UTF-8 must
    // still reach the resolver.
    if let Some(p) = env {
        if p.is_empty() {
            return Err(empty("CELLGOV_PS3_VFS_ROOT"));
        }
        return Ok(PathBuf::from(p));
    }
    Ok(PathBuf::from("vfs/dev_hdd0"))
}

#[cfg(test)]
#[path = "tests/title_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/title_registry_tests.rs"]
mod title_registry_tests;
