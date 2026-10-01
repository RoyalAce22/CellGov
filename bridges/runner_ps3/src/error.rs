//! The runner's one error type and the exit code each class maps to.

use std::path::PathBuf;

use cellgov_observation::console_profile::ConsoleProfileError;
use cellgov_observation::hardware_capture::HardwareCaptureError;
use cellgov_observation::manifest::ManifestError;

use crate::console::ConsoleError;
use crate::lease::LeaseError;
use crate::load::LoadError;

/// Process exit codes, one per failure class.
///
/// A script that drives the runner reads the class from the code alone;
/// the message names the particular.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::VariantArray)]
#[repr(i32)]
pub enum ExitCode {
    /// Every verb ran to completion.
    Ok = 0,
    /// The command line did not parse, or named an input that is missing
    /// or unreadable.
    Usage = 1,
    /// The runner refused before changing the console: a lease, a stale
    /// result, a profile mismatch, an existing output directory, a hot
    /// console or a full `/dev_hdd0`.
    Refused = 2,
    /// The console did not answer as the protocol requires.
    Transport = 3,
    /// The test did not leave its result within the manifest's budget.
    Timeout = 4,
    /// The fetched bytes are not one whole CGOV frame.
    Frame = 5,
    /// The capture succeeded but the runner could not restore the console.
    Cleanup = 6,
    /// The runner could not write its own output on this machine, or the
    /// host clock is unusable.
    Local = 7,
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
    /// An input file the verb names cannot be read.
    #[error("read {path}: {source}")]
    LocalRead {
        /// The file.
        path: PathBuf,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The verb could not write one of its output files.
    #[error("write {path}: {source}")]
    LocalWrite {
        /// The file or directory.
        path: PathBuf,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The host clock reads before 1970, so the capture has no honest
    /// timestamp.
    #[error("the host clock reads before 1970; set it before capturing")]
    HostClock,
    /// The manifest a verb names does not load.
    #[error("manifest: {0}")]
    Manifest(#[from] ManifestError),
    /// A committed capture does not load.
    #[error("committed capture: {0}")]
    Capture(#[from] HardwareCaptureError),
    /// A record the runner writes did not serialize.
    #[error("serialize: {0}")]
    Serialize(#[from] serde_json::Error),
    /// The runner refused before changing the console; the message names
    /// the command that clears it.
    #[error("refused: {reason}; clear it with `{clear_with}`")]
    Refused {
        /// What stands in the way.
        reason: String,
        /// The command that removes it.
        clear_with: String,
    },
    /// Another run holds the console, or the lease file failed.
    #[error("lease: {0}")]
    Lease(#[from] LeaseError),
    /// The console's identity is not established; the message names the
    /// field and what states it.
    #[error("console: {0}")]
    Console(#[from] ConsoleError),
    /// The profiles file did not load, or the console failed the claimed
    /// profile; the message names the profile to claim instead or the
    /// file to add one to.
    #[error("profile: {0}")]
    Profile(#[from] ConsoleProfileError),
    /// The thermal and capacity interlock refused: the console is hot,
    /// `/dev_hdd0` is full, or the page does not state a reading.
    #[error("load: {0}")]
    Load(#[from] LoadError),
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
            Self::Usage(_)
            | Self::LocalRead { .. }
            | Self::Manifest(_)
            | Self::Capture(_)
            | Self::Console(ConsoleError::OperatorMissing { .. }) => ExitCode::Usage,
            Self::LocalWrite { .. }
            | Self::HostClock
            | Self::Serialize(_)
            | Self::Lease(LeaseError::Io { .. }) => ExitCode::Local,
            Self::Refused { .. }
            | Self::Lease(LeaseError::Held { .. })
            | Self::Console(ConsoleError::Contradiction { .. }) => ExitCode::Refused,
            Self::Console(ConsoleError::PageMissing { .. }) => ExitCode::Transport,
            Self::Profile(
                ConsoleProfileError::Mismatch { .. } | ConsoleProfileError::UnknownProfile { .. },
            ) => ExitCode::Refused,
            Self::Profile(
                ConsoleProfileError::Io { .. }
                | ConsoleProfileError::Parse(_)
                | ConsoleProfileError::UnknownReference(_)
                | ConsoleProfileError::NoModels(_)
                | ConsoleProfileError::LoadLimits(_),
            ) => ExitCode::Usage,
            Self::Load(
                LoadError::Hot { .. } | LoadError::StillHot { .. } | LoadError::Full { .. },
            ) => ExitCode::Refused,
            Self::Transport(_) | Self::Load(LoadError::Unstated(_)) => ExitCode::Transport,
            Self::Timeout { .. } => ExitCode::Timeout,
            Self::Frame(_) => ExitCode::Frame,
            Self::Cleanup { .. } => ExitCode::Cleanup,
        }
    }
}

#[cfg(test)]
#[path = "tests/error_tests.rs"]
mod tests;
