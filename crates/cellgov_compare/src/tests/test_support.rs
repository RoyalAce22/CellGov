//! Shared fixtures for `cellgov_compare` unit tests.

use crate::observation::{
    NamedMemoryRegion, Observation, ObservationMetadata, ObservedEvent, ObservedEventKind,
    ObservedHashes, ObservedOutcome,
};
use cellgov_trace::StateHash;
use std::path::PathBuf;

/// Metadata tagged with the given runner name and no step count.
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

pub fn event(kind: ObservedEventKind, unit: u64, sequence: u32) -> ObservedEvent {
    ObservedEvent {
        kind,
        unit,
        sequence,
    }
}

/// Observation whose metadata is tagged `"test"` and whose state
/// hashes are absent.
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
/// The directory is removed recursively on drop, so a panicking test
/// does not leak temp state across runs.
///
/// `name` is the only thing separating one live guard from another
/// inside a single process, and the harness runs tests in parallel:
/// each call site must pass a name no other call site uses.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Create a fresh temp directory named `cellgov_<name>_<pid>`.
    ///
    /// # Panics
    ///
    /// If a stale directory at the same path cannot be removed, or the
    /// fresh one cannot be created.
    pub fn new(name: &str) -> Self {
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("cellgov_{name}_{pid}"));
        // Anything other than "it was not there" means the fixtures
        // this test writes would sit beside a previous run's files.
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("stale temp dir {} not removable: {e}", path.display()),
        }
        std::fs::create_dir_all(&path)
            .unwrap_or_else(|e| panic!("temp dir {} not creatable: {e}", path.display()));
        Self { path }
    }

    /// Absolute path of a file inside this temp directory.
    pub fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    #[allow(
        clippy::print_stderr,
        reason = "a drop-time cleanup refusal cannot panic; stderr is the only channel left"
    )]
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.path) {
            eprintln!("temp dir {} not removed on drop: {e}", self.path.display());
        }
    }
}
