//! The error type the store commands raise.

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
    #[cfg(feature = "decrypt")]
    #[error("serialize firmware kernel coverage: {source}")]
    KernelCoverageSerializeFailed {
        #[source]
        source: serde_json::Error,
    },
    #[cfg(feature = "decrypt")]
    #[error("create {}: {source}", path.display())]
    KernelCoverageDirCreateFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[cfg(feature = "decrypt")]
    #[error("write {}: {source}", path.display())]
    KernelCoverageWriteFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[cfg(not(feature = "decrypt"))]
    #[error("`{command}` decrypts, and this cellgov was built without the `decrypt` cargo feature; rebuild with `cargo build -p cellgov_cli --features decrypt`")]
    DecryptFeatureDisabled { command: String },
}
