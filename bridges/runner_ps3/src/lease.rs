//! One runner per console at a time: a lease file that names the
//! holder, refused with the command that releases it, and released on
//! drop.
//!
//! The caller names the directory the lease lives in, so every runner
//! on one machine must pass the same one.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

/// A held lease on one console.
#[derive(Debug)]
pub struct Lease {
    path: PathBuf,
    released: bool,
}

/// Why a lease was not taken or not released.
#[derive(Debug, thiserror::Error)]
pub enum LeaseError {
    /// Another runner, or a run that did not release, holds the console.
    #[error("{path} holds the console for {holder}; clear it with `{unlock_with}`")]
    Held {
        /// The lease file.
        path: PathBuf,
        /// The host the lease covers.
        host: String,
        /// What the lease file names as its holder.
        holder: String,
        /// The `unlock` command that clears it, through the front end
        /// that asked for the lease.
        unlock_with: String,
    },
    /// The runner cannot create, read or remove the lease file.
    #[error("lease file {path}: {source}")]
    Io {
        /// The lease file.
        path: PathBuf,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
}

/// The lease file for `host` under `dir`. Every character of the host
/// outside ASCII alphanumerics and `.` folds to `_`.
pub fn lease_path(dir: &Path, host: &str) -> PathBuf {
    let safe: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("cellgov_runner_ps3_{safe}.lease"))
}

impl Lease {
    /// Take the lease on `host` for `holder` (the microtest name).
    /// `invocation` is the front end's command, such as `runner_ps3`,
    /// which a refusal's `unlock` remedy starts with.
    ///
    /// # Errors
    ///
    /// [`LeaseError::Held`] when the file exists, naming its holder, and
    /// [`LeaseError::Io`] for any other failure.
    pub fn acquire(
        dir: &Path,
        host: &str,
        holder: &str,
        invocation: &str,
    ) -> Result<Self, LeaseError> {
        let path = lease_path(dir, host);
        let io = |source| LeaseError::Io {
            path: path.clone(),
            source,
        };
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let lease = Self {
                    path: path.clone(),
                    released: false,
                };
                writeln!(file, "pid={}\nholder={holder}", std::process::id()).map_err(io)?;
                Ok(lease)
            }
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                let text = std::fs::read_to_string(&path).map_err(io)?;
                Err(LeaseError::Held {
                    path: path.clone(),
                    host: host.to_string(),
                    holder: text.split_whitespace().collect::<Vec<_>>().join(" "),
                    unlock_with: format!("{invocation} unlock --host {host}"),
                })
            }
            Err(source) => Err(io(source)),
        }
    }

    /// The lease file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Remove the lease file.
    ///
    /// # Errors
    ///
    /// [`LeaseError::Io`] when the runner cannot remove the file.
    pub fn release(mut self) -> Result<(), LeaseError> {
        self.released = true;
        std::fs::remove_file(&self.path).map_err(|source| LeaseError::Io {
            path: self.path.clone(),
            source,
        })
    }
}

impl Drop for Lease {
    /// Remove the file on an early return or a panic. [`Lease::release`]
    /// is the path that reports a failure; a drop cannot, and `unlock`
    /// clears what one leaves behind.
    fn drop(&mut self) {
        if !self.released {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Remove the lease on `host` whoever holds it. Returns whether a lease
/// was there.
///
/// # Errors
///
/// [`LeaseError::Io`] for a failure other than an absent file.
pub fn unlock(dir: &Path, host: &str) -> Result<bool, LeaseError> {
    let path = lease_path(dir, host);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(LeaseError::Io { path, source }),
    }
}

#[cfg(test)]
#[path = "tests/lease_tests.rs"]
mod tests;
