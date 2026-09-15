//! The cell an implicitly selected firmware puts a run in.

use std::path::PathBuf;

use super::*;
use crate::composition::compose::StoredGame;
use crate::composition::inventory::{BaseEntry, FirmwareEntry};
use crate::composition::select::{FirmwareSelectedBy, ManagedFirmware};
use cellgov_boot::manifest::{
    CellExpectation, CheckpointTrigger, Distribution, GameSource, MatrixCell, TitleManifest,
};

const DISC: &str = "BLAA00001";

const SHIPPED_FW: &str = "3.55";

fn disc_composition(selected_by: FirmwareSelectedBy) -> BootComposition {
    BootComposition {
        firmware: FirmwareChoice::Managed(ManagedFirmware {
            entry: FirmwareEntry {
                version: SHIPPED_FW.to_string(),
                entry_dir: PathBuf::from("store/firmware/3.55"),
                pup_sha256: "0".repeat(64),
            },
            selected_by,
        }),
        game: GameChoice::Stored(Box::new(StoredGame {
            title_id: DISC.to_string(),
            version: GameVersion::Base,
            base: BaseEntry {
                version: "01.00".to_string(),
                dir: PathBuf::from("store/titles/BLAA00001/base/disc"),
                tree: cellgov_install::store::TitleTree::Disc,
                distribution: "disc-iso".to_string(),
                source_sha256: "0".repeat(64),
                system_ver: Some("03.5500".to_string()),
                shipped_firmware: Some(SHIPPED_FW.to_string()),
            },
            update: None,
        })),
        mounts: Vec::new(),
        eboot_dirs: Vec::new(),
        understated_firmware: Vec::new(),
        identity: cellgov_compare::RunIdentity::default(),
    }
}

fn shipped_cell() -> CellKey {
    CellKey {
        fw: SHIPPED_FW.to_string(),
        game_ver: Some(BASE_GAME_VER.to_string()),
    }
}

/// A disc manifest whose one cell, shipped firmware times base,
/// overrides the manifest's cap and checkpoint.
fn disc_manifest_declaring(bench_max_steps: u64, checkpoint: CheckpointTrigger) -> TitleManifest {
    TitleManifest {
        content_id: DISC.to_string(),
        short_name: "t".to_string(),
        display_name: "Synthetic".to_string(),
        eboot_candidates: vec!["EBOOT.BIN".to_string()],
        year: 2007,
        developer: "test-developer".to_string(),
        engine: "test-engine".to_string(),
        distribution: Distribution::DiscIso,
        rap_filename: None,
        bench_max_steps: Some(250_000_000),
        system_ver: Some(SHIPPED_FW.to_string()),
        checkpoint: CheckpointTrigger::ProcessExit,
        source: GameSource::Disc,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
        matrix: vec![MatrixCell {
            key: shipped_cell(),
            expect: CellExpectation::Frontier,
            bench_max_steps: Some(bench_max_steps),
            checkpoint: Some(checkpoint),
            pending: None,
        }],
    }
}

#[test]
fn a_shipped_firmware_keys_the_cell_like_a_named_one() {
    let named = composed_cell(&disc_composition(FirmwareSelectedBy::Flag));
    assert_eq!(named, Some(shipped_cell()));
    assert_eq!(
        composed_cell(&disc_composition(FirmwareSelectedBy::Shipped)),
        named
    );
}

#[test]
fn a_sole_firmware_keys_the_cell_like_a_named_one() {
    assert_eq!(
        composed_cell(&disc_composition(FirmwareSelectedBy::Sole)),
        composed_cell(&disc_composition(FirmwareSelectedBy::Flag)),
    );
}

#[test]
fn a_shipped_firmware_takes_the_declared_cells_cap_and_checkpoint() {
    let title = disc_manifest_declaring(4_000, CheckpointTrigger::FirstRsxWrite);
    let plan = ResolvedPlan::resolve(&title, &disc_composition(FirmwareSelectedBy::Shipped));
    assert_eq!(plan.cell, Some(shipped_cell()));
    assert_eq!(plan.max_steps, 4_000);
    assert_eq!(plan.checkpoint, CheckpointTrigger::FirstRsxWrite);
    assert_eq!(plan.max_steps_usize(&title), 4_000);
}
