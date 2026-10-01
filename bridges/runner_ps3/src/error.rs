//! The runner's one error type and the exit code each class maps to.

use std::path::PathBuf;

use cellgov_compare::console_profile::ConsoleProfileError;

/// Process exit codes, one per failure class.
///
/// A script that drives the runner reads the class from the code alone;
/// the message names the particular.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::VariantArray)]
#[repr(i32)]
pub enum ExitCode {
    /// Every verb ran to completion.
    Ok = 0,
    /// The command line did not parse or named a missing input.
    Usage = 1,
    /// The runner refused before touching the console: a lease, a stale
    /// result, a profile mismatch, an existing output directory.
    Refused = 2,
    /// The console did not answer as the protocol requires.
    Transport = 3,
    /// The test did not leave its result within the manifest's budget.
    Timeout = 4,
    /// The fetched bytes are not one whole CGOV frame.
    Frame = 5,
    /// The capture succeeded but the runner could not restore the console.
    Cleanup = 6,
}

impl ExitCode {
    /// The process exit status.
    pub fn code(self) -> i32 {
        self as i32
    }
}

/// Why a runner verb failed.
#[derive(Debug, thiserror::Error)]
pub enum RunnerPs3Error {
    /// The command line did not parse.
    #[error("usage: {0}")]
    Usage(String),
    /// A local file the verb needs cannot be read or written.
    #[error("{path}: {source}")]
    LocalIo {
        /// The file.
        path: PathBuf,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The runner refused before touching the console; the message
    /// names the command that clears it.
    #[error("refused: {reason}; clear it with `{clear_with}`")]
    Refused {
        /// What stands in the way.
        reason: String,
        /// The command that removes it.
        clear_with: String,
    },
    /// The profiles file did not load, or the console failed the claimed
    /// profile; the message names the profile to claim instead or the
    /// file to add one to.
    #[error("profile: {0}")]
    Profile(#[from] ConsoleProfileError),
    /// The console did not answer as the protocol requires.
    #[error("transport: {0}")]
    Transport(#[from] crate::transport::TransportError),
    /// The result did not appear within the budget.
    #[error("timeout: no result at {result_path} after {timeout_ms} ms")]
    Timeout {
        /// Where the test writes its frame.
        result_path: String,
        /// The manifest's budget.
        timeout_ms: u64,
    },
    /// The fetched bytes are not one whole CGOV frame.
    #[error("frame: {0}")]
    Frame(String),
    /// Cleanup after a capture left something on the console.
    #[error("cleanup: {remaining} remains on the console")]
    Cleanup {
        /// What the runner could not remove.
        remaining: String,
    },
}

impl RunnerPs3Error {
    /// The exit code for this error's class.
    pub fn exit_code(&self) -> ExitCode {
        match self {
            Self::Usage(_) | Self::LocalIo { .. } => ExitCode::Usage,
            Self::Refused { .. } => ExitCode::Refused,
            Self::Profile(
                ConsoleProfileError::Mismatch { .. } | ConsoleProfileError::UnknownProfile { .. },
            ) => ExitCode::Refused,
            Self::Profile(
                ConsoleProfileError::Io { .. }
                | ConsoleProfileError::Parse(_)
                | ConsoleProfileError::UnknownReference(_)
                | ConsoleProfileError::NoModels(_),
            ) => ExitCode::Usage,
            Self::Transport(_) => ExitCode::Transport,
            Self::Timeout { .. } => ExitCode::Timeout,
            Self::Frame(_) => ExitCode::Frame,
            Self::Cleanup { .. } => ExitCode::Cleanup,
        }
    }
}

#[cfg(test)]
#[path = "tests/error_tests.rs"]
mod tests;
