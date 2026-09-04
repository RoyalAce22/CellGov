//! Advisory locks that keep two writers off one store artifact.
//!
//! # Invariants
//!
//! - The open file handle is the claim: a second handle on one lock
//!   file conflicts with the first, even inside one process.
//!   `std::fs::File::try_lock` reports that as `WouldBlock`.
//! - The host releases the claim when the handle closes, so a
//!   terminated writer blocks nothing. Win32 releases the locks of a
//!   killed process on its own schedule. A reader that waits on a
//!   killed holder must retry.
//! - A writer creates a lock file and never removes it.
//! - Contention refuses rather than blocks, so no acquisition order can
//!   deadlock.

use std::fs::{File, TryLockError};
use std::path::PathBuf;

use crate::store::layout::{Artifact, StoreLayout};

/// What a refusal calls the firmware staging directory.
const FIRMWARE_STAGING: &str = "the firmware staging directory";

/// Why a store artifact could not be claimed.
#[derive(Debug, thiserror::Error)]
pub enum StoreLockError {
    /// Another open handle holds the artifact.
    #[error(
        "{held} is being installed or removed by another writer; the lock is {}",
        path.display()
    )]
    Contended {
        /// What the lock names.
        held: String,
        /// The lock file the other writer holds.
        path: PathBuf,
    },
    /// A filesystem operation failed on the lock file, or on the
    /// directory holding it.
    #[error("{op} {}: {source}", path.display())]
    Io {
        /// What was attempted, phrased to read against `path`.
        op: &'static str,
        /// The path involved: the lock file, or its parent directory
        /// when it is the directory that could not be made.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
}

/// An exclusive claim on one store artifact, released when dropped.
#[derive(Debug)]
#[must_use = "the claim is released as soon as this value is dropped"]
pub struct StoreLock {
    /// The handle whose lifetime is the claim.
    _file: File,
}

/// Claim `artifact` until the returned value is dropped.
///
/// # Errors
///
/// [`StoreLockError::Contended`] when another open handle holds the
/// artifact, and [`StoreLockError::Io`] when the lock file cannot be
/// created or locked.
pub fn lock_artifact(
    layout: &StoreLayout,
    artifact: &Artifact,
) -> Result<StoreLock, StoreLockError> {
    acquire(layout.lock_path(artifact), describe(artifact))
}

/// Claim the staging directory every firmware install extracts into.
///
/// # Errors
///
/// The same refusals as [`lock_artifact`].
pub fn lock_firmware_staging(layout: &StoreLayout) -> Result<StoreLock, StoreLockError> {
    acquire(
        layout.firmware_staging_lock_path(),
        FIRMWARE_STAGING.to_string(),
    )
}

/// What a refusal calls `artifact`.
fn describe(artifact: &Artifact) -> String {
    match artifact {
        Artifact::Firmware { version } => format!("firmware {}", version.as_str()),
        Artifact::TitleBase { title_id } => format!("the base install of {}", title_id.as_str()),
        Artifact::TitleUpdate { title_id, version } => {
            format!("update {} of {}", version.as_str(), title_id.as_str())
        }
    }
}

fn acquire(path: PathBuf, held: String) -> Result<StoreLock, StoreLockError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| StoreLockError::Io {
            op: "create the lock directory",
            path: parent.to_path_buf(),
            source,
        })?;
    }
    // `truncate(false)`: nothing reads the content, and a truncate
    // would write to a file another writer holds open and locked.
    //
    // `read(true).write(true)`: Win32 refuses to lock a handle opened
    // for append alone. `std::fs::File::lock` needs read or write
    // access on the handle.
    let file = File::options()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|source| StoreLockError::Io {
            op: "open the lock file",
            path: path.clone(),
            source,
        })?;
    // `try_lock` never reports contention through the `Error` arm. The
    // standard library keeps `ErrorKind::WouldBlock` out of it, so the
    // two arms below do not overlap.
    match file.try_lock() {
        Ok(()) => Ok(StoreLock { _file: file }),
        Err(TryLockError::WouldBlock) => Err(StoreLockError::Contended { held, path }),
        Err(TryLockError::Error(source)) => Err(StoreLockError::Io {
            op: "take the lock on",
            path,
            source,
        }),
    }
}

#[cfg(test)]
#[path = "tests/lock_tests.rs"]
mod tests;
