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
    /// Installing the system software a disc image ships, as a firmware
    /// entry of its own. Runs for minutes.
    InstallingFirmware = 7,
}

impl Phase {
    /// The code a [`ProgressSink`] stores.
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }
}

/// A coarse stage of a firmware install.
///
/// [`Self::Extracting`] is the measured stage, denominated in the
/// dev_flash packages' payload bytes; the others are indeterminate. It
/// covers each package's decrypt as well as its write, since a renderer
/// draws a determinate bar only while the measured phase is current.
/// The discriminants index [`FIRMWARE_TASK`]'s label table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FirmwarePhase {
    /// Parsing the PUP container's tables.
    Reading = 0,
    /// Recomputing every payload's HMAC.
    ValidatingHmac = 1,
    /// Removing the staging tree an interrupted install left behind.
    ClearingStaging = 2,
    /// Opening each dev_flash package and writing its files into the
    /// staging tree.
    Extracting = 3,
    /// Hashing the staged modules into `firmware.toml`.
    BuildingManifest = 4,
    /// Clearing an existing entry directory (`force` overwrite).
    Clearing = 5,
    /// The commit rename sequence.
    Committing = 6,
    /// Opening the CoreOS package and writing the kernel beside the
    /// tree.
    UnpackingKernel = 7,
}

impl FirmwarePhase {
    /// The code a [`ProgressSink`] stores.
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }
}

/// How a renderer presents a firmware install.
pub const FIRMWARE_TASK: Task = Task {
    verb: "Installing",
    tag: "firmware",
    phases: &[
        "reading PUP",
        "validating HMAC",
        "clearing staging",
        "decrypting packages",
        "building manifest",
        "clearing old install",
        "committing",
        "unpacking kernel",
    ],
    measured: FirmwarePhase::Extracting as u8,
    unit: Unit::Bytes,
    items: "packages",
    streaming: false,
};

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
        "installing shipped firmware",
    ],
    measured: Phase::Staging as u8,
    unit: Unit::Bytes,
    items: "files",
    streaming: false,
};

#[cfg(test)]
#[path = "tests/progress_tests.rs"]
mod tests;
