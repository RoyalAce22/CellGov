//! Aggregate [`Observation`] and [`ObservationMetadata`] structs.
//! Constituent types live in sibling submodules ([`super::event`],
//! [`super::hashes`], [`super::memory`], [`super::outcome`]).

use serde::{Deserialize, Serialize};

use crate::observation::event::ObservedEvent;
use crate::observation::hashes::ObservedHashes;
use crate::observation::memory::NamedMemoryRegion;
use crate::observation::outcome::ObservedOutcome;

/// Per-run metadata recorded alongside an [`Observation`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationMetadata {
    /// Runner identifier (e.g. `"cellgov"`, `"cellgov-boot"`,
    /// `"rpcs3-interpreter"`). Compared verbatim in cross-runner
    /// diffs, so do not include host-environment noise.
    pub runner: String,
    /// `None` when the runner does not expose a step count.
    pub steps: Option<usize>,
}

/// Normalized observation from a single test run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    /// How the run terminated.
    pub outcome: ObservedOutcome,
    /// End-of-run snapshots of the regions declared in the manifest,
    /// in manifest order.
    pub memory_regions: Vec<NamedMemoryRegion>,
    /// Observable events emitted during the run, in dispatch order.
    pub events: Vec<ObservedEvent>,
    /// `None` for runners that do not expose internal state hashes
    /// (e.g., RPCS3).
    pub state_hashes: Option<ObservedHashes>,
    /// Per-run metadata: runner identity and optional step count.
    pub metadata: ObservationMetadata,
    /// `sys_tty_write` byte stream in dispatch order; empty when no
    /// TTY output was captured.
    #[serde(default)]
    pub tty_log: Vec<u8>,
}

#[cfg(test)]
#[path = "tests/model_tests.rs"]
mod tests;
