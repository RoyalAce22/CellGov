//! The installed vault under a VFS root: `keys import` writes it,
//! `keys remove` deletes it.

use std::path::{Path, PathBuf};

use super::{installed_keys_dir, KeyVault, KeyVaultError, INSTALLED_KEYS_FILE};

/// What [`import_into`] wrote.
#[derive(Debug)]
pub struct ImportOutcome {
    /// The vault now installed.
    pub vault: KeyVault,
    /// Whether the import merged into a vault that was already
    /// installed, rather than writing a fresh one.
    pub merged: bool,
    /// The installed `keys.toml`.
    pub file: PathBuf,
}

/// Why [`import_into`] wrote nothing.
#[derive(Debug, thiserror::Error)]
pub enum KeyImportError {
    /// The imported vault would not load, or a merge found two
    /// definitions of one key that disagree.
    #[error(transparent)]
    Vault(#[from] KeyVaultError),
    /// The imported path held no key a decrypt path could use.
    #[error("{} holds no key", path.display())]
    NothingUsable {
        /// The imported path.
        path: PathBuf,
    },
    /// Creating the installed-vault directory failed.
    #[error("create {}: {source}", path.display())]
    DirCreate {
        /// The directory.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// Writing `keys.toml` failed.
    #[error("write {}: {source}", path.display())]
    Write {
        /// The file.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
}

/// Why [`remove_installed`] could not delete the installed vault.
#[derive(Debug, thiserror::Error)]
#[error("remove {}: {source}", path.display())]
pub struct KeyRemoveError {
    /// The installed-vault directory.
    pub path: PathBuf,
    /// The underlying failure.
    #[source]
    pub source: std::io::Error,
}

/// Normalize the vault at `path` into `<vfs_root>/.cellgov/keys/keys.toml`.
///
/// Without `replace`, the import merges into the vault already there.
/// A refused import leaves the installed vault untouched.
///
/// # Errors
///
/// - [`KeyImportError::Vault`] for a vault that will not load, or for a
///   merge whose two definitions of one key disagree. The refusal names
///   both definitions.
/// - [`KeyImportError::NothingUsable`] when `path` held no key.
/// - [`KeyImportError::DirCreate`] and [`KeyImportError::Write`] for the
///   write refusals.
pub fn import_into(
    path: &Path,
    vfs_root: &Path,
    replace: bool,
) -> Result<ImportOutcome, KeyImportError> {
    let imported = KeyVault::load_from_path(path)?;
    if !imported.holds_any_key() {
        return Err(KeyImportError::NothingUsable {
            path: path.to_path_buf(),
        });
    }
    let dir = installed_keys_dir(vfs_root);
    let file = dir.join(INSTALLED_KEYS_FILE);
    let merged = !replace && file.is_file();
    let vault = if merged {
        let mut existing = KeyVault::load_from_path(&file)?;
        existing.merge(imported)?;
        existing
    } else {
        imported
    };
    std::fs::create_dir_all(&dir).map_err(|source| KeyImportError::DirCreate {
        path: dir.clone(),
        source,
    })?;
    std::fs::write(&file, vault.to_toml()).map_err(|source| KeyImportError::Write {
        path: file.clone(),
        source,
    })?;
    Ok(ImportOutcome {
        vault,
        merged,
        file,
    })
}

/// Delete `<vfs_root>/.cellgov/keys/`.
///
/// Returns `Ok(false)` when there was nothing to delete.
///
/// # Errors
///
/// [`KeyRemoveError`] for any refusal other than absence.
pub fn remove_installed(vfs_root: &Path) -> Result<bool, KeyRemoveError> {
    let dir = installed_keys_dir(vfs_root);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(KeyRemoveError { path: dir, source }),
    }
}

#[cfg(test)]
#[path = "tests/installed_tests.rs"]
mod tests;
