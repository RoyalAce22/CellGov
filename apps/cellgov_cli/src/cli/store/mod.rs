//! Handlers for the commands that read and write the content store:
//! `firmware`, `title`, `keys`, and `self decrypt`.

pub(crate) mod firmware;
pub(crate) mod keys_cmd;
#[cfg(feature = "decrypt")]
pub(crate) mod rap;
pub(crate) mod self_decrypt;
pub(crate) mod title;

#[cfg(test)]
#[path = "tests/scratch.rs"]
pub(crate) mod scratch;

#[cfg(test)]
#[path = "tests/store_tests.rs"]
mod tests;

#[cfg(feature = "decrypt")]
use std::path::Path;
use std::path::PathBuf;

/// Why a store command's own resolution failed, before it reaches the
/// library it drives.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StoreCliError {
    /// A RAP file is not the 16 bytes the klicensee derivation needs.
    #[cfg(feature = "decrypt")]
    #[error("RAP {} is {len} bytes; expected exactly 16", path.display())]
    RapWrongSize { path: PathBuf, len: usize },
    /// A RAP file exists but could not be read.
    #[cfg(feature = "decrypt")]
    #[error("read RAP {}: {source}", path.display())]
    RapReadFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// `--rap` named a file that is not there. Distinct from the
    /// exdata probe, whose miss is the ordinary uninstalled case.
    #[cfg(feature = "decrypt")]
    #[error("--rap {} does not exist", path.display())]
    ExplicitRapMissing { path: PathBuf },
    /// Loading, merging, or locating a vault failed.
    #[error("{0}")]
    Keys(#[from] cellgov_install::keys::KeyVaultError),
    #[error("keys import: {} holds no scalar key, SCE package keyset, or APP/NPDRM keyset; nothing to import (run `keys show {}` to see what was read and set aside)", path.display(), path.display())]
    KeysNothingUsable { path: PathBuf },
    /// `keys import`: the installed-vault directory could not be
    /// created.
    #[error("create {}: {source}", path.display())]
    KeysDirCreateFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// `keys import`: `keys.toml` could not be written.
    #[error("write {}: {source}", path.display())]
    KeysWriteFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// `keys remove`: the installed-vault directory exists and could
    /// not be deleted.
    #[error("remove {}: {source}", path.display())]
    KeysRemoveFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[cfg(not(feature = "decrypt"))]
    #[error("`{command}` decrypts, and this cellgov was built without the `decrypt` cargo feature; rebuild with `cargo build -p cellgov_cli --features decrypt`")]
    DecryptFeatureDisabled { command: String },
}

/// The container's filename, for the progress bar's title line.
#[cfg(feature = "decrypt")]
pub(crate) fn container_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Map a container for a sequential read.
///
/// A disc image can exceed host RAM. Every installer reads its
/// container in sequence, so the host streams pages in and evicts them.
#[cfg(feature = "decrypt")]
pub(crate) fn map_container_or_die(path: &Path) -> filebuffer::FileBuffer {
    filebuffer::FileBuffer::open(path).unwrap_or_else(|e| {
        crate::cli::exit::die(&format!("failed to map {}: {e}", path.display()))
    })
}

/// The megabyte figure the banner lines print.
#[cfg(feature = "decrypt")]
pub(crate) fn megabytes(len: usize) -> f64 {
    len as f64 / (1024.0 * 1024.0)
}

/// Load the operator's vault for `store`, or die naming the refusal.
#[cfg(feature = "decrypt")]
pub(crate) fn vault_or_die(store: &Path) -> cellgov_install::keys::KeyVault {
    cellgov_install::keys::KeyVault::load_for_vfs(store)
        .unwrap_or_else(|e| crate::cli::exit::die(&e.to_string()))
}
