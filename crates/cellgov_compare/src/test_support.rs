//! Shared fixtures for `cellgov_compare` unit tests.

use crate::observation::{
    NamedMemoryRegion, Observation, ObservationMetadata, ObservedEvent, ObservedEventKind,
    ObservedHashes, ObservedOutcome,
};
use cellgov_trace::StateHash;
use std::path::PathBuf;

/// Build observation metadata tagged with the given runner name.
pub fn meta(runner: &str) -> ObservationMetadata {
    ObservationMetadata {
        runner: runner.into(),
        steps: None,
    }
}

/// Build a named memory region at a canonical test address.
pub fn region(name: &str, data: Vec<u8>) -> NamedMemoryRegion {
    NamedMemoryRegion {
        name: name.into(),
        addr: 0x10000,
        data,
    }
}

/// Build an observed event with the given kind, unit, and sequence.
pub fn event(kind: ObservedEventKind, unit: u64, sequence: u32) -> ObservedEvent {
    ObservedEvent {
        kind,
        unit,
        sequence,
    }
}

/// Build an observation with the given outcome, regions, and events.
///
/// Metadata is tagged `"test"` and state hashes are absent. Tests that
/// need hashes should use [`sample_observation`] or construct
/// [`Observation`] directly.
pub fn obs(
    outcome: ObservedOutcome,
    regions: Vec<NamedMemoryRegion>,
    events: Vec<ObservedEvent>,
) -> Observation {
    Observation {
        outcome,
        memory_regions: regions,
        events,
        state_hashes: None,
        metadata: meta("test"),
        tty_log: Vec::new(),
    }
}

/// A fully populated observation for baseline roundtrip and equality tests.
pub fn sample_observation() -> Observation {
    Observation {
        outcome: ObservedOutcome::Completed,
        memory_regions: vec![NamedMemoryRegion {
            name: "result".into(),
            addr: 0x10000,
            data: vec![0, 0, 0, 1],
        }],
        events: vec![
            ObservedEvent {
                kind: ObservedEventKind::MailboxSend,
                unit: 0,
                sequence: 0,
            },
            ObservedEvent {
                kind: ObservedEventKind::UnitWake,
                unit: 1,
                sequence: 1,
            },
            ObservedEvent {
                kind: ObservedEventKind::MailboxReceive,
                unit: 1,
                sequence: 2,
            },
        ],
        state_hashes: Some(ObservedHashes {
            memory: StateHash::new(0xaabb_ccdd_eeff_0011),
            unit_status: StateHash::new(0x1122_3344_5566_7788),
            sync: StateHash::new(0x99aa_bbcc_ddee_ff00),
        }),
        metadata: ObservationMetadata {
            runner: "cellgov".into(),
            steps: Some(42),
        },
        tty_log: b"sample tty\n".to_vec(),
    }
}

/// RAII guard for a per-test temp directory under `std::env::temp_dir()`.
///
/// Removes the directory recursively on drop so panicking tests do not
/// leak temp state across runs.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Create a fresh temp directory named `cellgov_<name>`.
    pub fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("cellgov_{name}"));
        std::fs::remove_dir_all(&path).ok();
        std::fs::create_dir_all(&path).ok();
        Self { path }
    }

    /// Absolute path of a file inside this temp directory.
    pub fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}
