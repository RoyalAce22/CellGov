//! The CLI's key vault: loaded on the first SELF that needs it, once.

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use cellgov_install::keys::{KeyVault, KeyVaultError};
use cellgov_install::self_image::is_sce_wrapped;

static NO_KEYS: KeyVault = KeyVault::empty();

/// The install root [`key_vault`] reads the imported vault under,
/// fixed by [`fix_vault_root`] before the first SCE-wrapped image.
static VAULT_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// The install root enclosing the PS3 VFS root a subcommand was given.
///
/// `--vfs-root` names the `dev_hdd0` mount, while `cellgov_install`
/// writes the vault, the install records and the `dev_flash` mount one
/// level up; the two must agree or an operator who installed under a
/// non-default root gets `NotConfigured` from a vault
/// `cellgov_install keys show` finds.
fn install_root_of(ps3_vfs_root: &Path) -> PathBuf {
    let mut comps = ps3_vfs_root.components();
    match comps.next_back() {
        // Dropping a name is the only case where the remaining
        // components spell the directory above.
        Some(Component::Normal(_)) => {
            let above = comps.as_path();
            // A bare relative name (`--vfs-root dev_hdd0`) leaves
            // nothing, which as a path names the filesystem root
            // rather than the working directory the name was relative
            // to.
            if above.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                above.to_path_buf()
            }
        }
        // `Path::parent` is lexical: it drops a trailing `.` or `..`
        // like any other name, and a drive-relative prefix (`C:`, no
        // root) has no parent component at all. For these three the
        // enclosing directory has no spelling in the path other than
        // `..` joined onto it.
        Some(Component::CurDir | Component::ParentDir | Component::Prefix(_)) => {
            ps3_vfs_root.join("..")
        }
        // A filesystem root encloses nothing, so it stands in for its
        // own parent.
        Some(Component::RootDir) | None => ps3_vfs_root.to_path_buf(),
    }
}

/// Fix the install root the operator's vault is read under, from the
/// PS3 VFS root the subcommand resolved.
///
/// Called from [`super::title::resolve_ps3_vfs_root`], which every
/// subcommand that opens a guest image goes through before it opens
/// one. The loaders inside a boot (`game::boot`, `game::prx::load`)
/// have no root in hand and read what was fixed here.
///
/// # Panics
///
/// A second call naming a different root: the vault may already have
/// been loaded under the first, so the later root would apply to some
/// images in the run and not others.
pub(crate) fn fix_vault_root(ps3_vfs_root: &Path) {
    fix_vault_root_in(&VAULT_ROOT, ps3_vfs_root);
}

/// [`fix_vault_root`] against an explicit cell, so a test can drive
/// the conflict arm without settling the process-wide one.
fn fix_vault_root_in(cell: &OnceLock<PathBuf>, ps3_vfs_root: &Path) {
    let root = install_root_of(ps3_vfs_root);
    let fixed = cell.get_or_init(|| root.clone());
    assert!(
        *fixed == root,
        "key vault root already fixed at {} and cannot become {}",
        fixed.display(),
        root.display(),
    );
}

/// The root [`fix_vault_root`] settled on, or `None` while nothing has
/// fixed or loaded one.
#[cfg(test)]
pub(crate) fn fixed_vault_root() -> Option<&'static Path> {
    VAULT_ROOT.get().map(PathBuf::as_path)
}

/// The vault `bytes` decrypts under: the operator's for an SCE
/// wrapper, an empty one for a plaintext image; a vault that will not
/// load dies naming the cause.
pub(crate) fn key_vault_for(bytes: &[u8]) -> &'static KeyVault {
    try_key_vault_for(bytes).unwrap_or_else(|e| super::exit::die(&format!("key vault: {e}")))
}

/// [`key_vault_for`] returning the load refusal instead of dying, for
/// a loader that runs inside `Runtime::step`.
pub(crate) fn try_key_vault_for(bytes: &[u8]) -> Result<&'static KeyVault, &'static KeyVaultError> {
    if is_sce_wrapped(bytes) {
        key_vault()
    } else {
        Ok(&NO_KEYS)
    }
}

/// The vault every SELF open in this process decrypts under, loaded
/// on first use; a load refusal is kept and answered the same way to
/// every later caller.
///
/// A subcommand that names no VFS root falls back to
/// `cellgov_install`'s own default install root, so the two agree
/// there too.
#[cfg(feature = "decrypt")]
fn key_vault() -> Result<&'static KeyVault, &'static KeyVaultError> {
    static VAULT: OnceLock<Result<KeyVault, KeyVaultError>> = OnceLock::new();
    VAULT
        .get_or_init(|| {
            let default = || PathBuf::from(cellgov_install::game_install::DEFAULT_VFS_ROOT);
            KeyVault::load_for_vfs(VAULT_ROOT.get_or_init(default))
        })
        .as_ref()
}

/// Without the `decrypt` feature the vault is never consulted: an
/// SCE-wrapped image is refused by the feature before any key lookup.
#[cfg(not(feature = "decrypt"))]
fn key_vault() -> Result<&'static KeyVault, &'static KeyVaultError> {
    Ok(&NO_KEYS)
}

#[cfg(test)]
#[path = "tests/keys_tests.rs"]
mod tests;
