//! Shared fixtures for `cellgov_observation` unit tests.

use cellgov_trace::StateHash;

use crate::identity::{AppVersion, FirmwareIdentity, GameIdentity, RunIdentity};
use crate::observation::{
    NamedMemoryRegion, Observation, ObservationMetadata, ObservedEvent, ObservedEventKind,
    ObservedHashes, ObservedOutcome, CHECKPOINT_HASH_SCHEME,
};

/// A fully populated observation for round-trip and equality tests.
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
            scheme: CHECKPOINT_HASH_SCHEME,
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
        overrides: Default::default(),
    }
}
