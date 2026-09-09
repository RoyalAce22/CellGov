//! Shared fixtures for `cellgov_compare` unit tests.

use crate::identity::{AppVersion, FirmwareIdentity, GameIdentity, RunIdentity};
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
        identity: RunIdentity::default(),
        runner_firmware: None,
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
        identity: identity("4.91", "NPAA00001", "base"),
        runner_firmware: None,
    }
}

/// A fully populated identity triple, with the image version derived
/// from `fw`.
pub fn identity(fw: &str, title_id: &str, version: &str) -> RunIdentity {
    RunIdentity {
        firmware: Some(FirmwareIdentity {
            version: fw.into(),
            image_version: format!("0x{}", fw.replace('.', "")),
            pup_sha256: "00".repeat(32),
        }),
        game: Some(GameIdentity {
            title_id: title_id.into(),
            version: version.into(),
            app_version: Some(AppVersion::AppVer("02.00".into())),
        }),
    }
}

/// A per-test temp directory, removed on drop.
///
/// Two guards under one `name` still get separate directories.
pub struct TempDir {
    dir: cellgov_testkit::scratch::ScratchDir,
}

impl TempDir {
    /// A fresh temp directory whose name carries `name`.
    ///
    /// # Panics
    ///
    /// If the directory cannot be created.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            dir: cellgov_testkit::scratch::scratch_labeled(name),
        }
    }

    /// Absolute path of a file inside this temp directory.
    pub fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

#[test]
fn two_guards_under_one_name_are_separate_directories() {
    let first = TempDir::new("duplicate_name");
    let marker = first.file("marker");
    std::fs::write(&marker, b"first").expect("the first guard's fixture");

    let second = TempDir::new("duplicate_name");
    assert_ne!(
        second.file("marker"),
        marker,
        "one name resolved to one path"
    );
    assert_eq!(
        std::fs::read(&marker).expect("the first guard's fixture survives the second guard"),
        b"first".as_slice(),
        "the second guard wiped the first one's fixtures"
    );
}
