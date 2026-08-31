//! The install's progress vocabulary: its stages, and the task
//! descriptor a renderer lays them out from.
//!
//! The sink itself is [`cellgov_terminal::progress::ProgressSink`];
//! only the labels are the installer's.

use cellgov_terminal::progress::{Task, Unit};

pub use cellgov_terminal::progress::ProgressSink;

/// A coarse stage of an install, for a reporter to label its output.
///
/// [`Self::Staging`] is the only stage with a byte denominator; the
/// others have no natural progress unit and a renderer treats them as
/// indeterminate. The discriminants index [`INSTALL_TASK`]'s label
/// table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Phase {
    /// Reading and validating the container's structure.
    Reading = 0,
    /// Writing the game tree into the staging root.
    Staging = 1,
    /// Running the decrypt-proof gate.
    Proving = 2,
    /// Clearing an existing target directory (`force` overwrite).
    Clearing = 3,
    /// The commit rename sequence.
    Committing = 4,
    /// Hashing the source container for the install record.
    Hashing = 5,
    /// Removing the staging tree an interrupted install left behind.
    ClearingStaging = 6,
}

impl Phase {
    /// The code a [`ProgressSink`] stores.
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }
}

/// How a renderer presents an install.
pub const INSTALL_TASK: Task = Task {
    verb: "Installing",
    tag: "install",
    phases: &[
        "reading",
        "staging",
        "verifying decrypt",
        "clearing old install",
        "committing",
        "hashing source",
        "clearing staging",
    ],
    measured: Phase::Staging as u8,
    unit: Unit::Bytes,
    items: "files",
    streaming: false,
};

#[cfg(test)]
#[path = "tests/progress_tests.rs"]
mod tests;
