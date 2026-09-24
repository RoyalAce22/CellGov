//! The cell a composition puts a run in, and what the registry
//! declares for it.

use std::path::PathBuf;

use super::*;
use crate::composition::compose::StoredGame;
use crate::composition::select::{FirmwareSelectedBy, ManagedFirmware};
use cellgov_boot::manifest::{CellExpectation, MatrixCell};
use cellgov_install::store::inventory::{BaseEntry, FirmwareEntry};

fn manifest(
    bench_max_steps: Option<u64>,
    matrix: Vec<MatrixCell>,
) -> cellgov_boot::manifest::TitleManifest {
    use cellgov_boot::manifest::{CheckpointTrigger, Distribution, GameSource, TitleManifest};
    TitleManifest {
        content_id: "CG_TEST".to_string(),
        short_name: "test".to_string(),
        display_name: "test".to_string(),
        eboot_candidates: vec!["EBOOT.BIN".to_string()],
        year: 2007,
        developer: "test-developer".to_string(),
        engine: "test-engine".to_string(),
        distribution: Distribution::PsnHdd,
        rap_filename: None,
        bench_max_steps,
        system_ver: Some("4.93".to_string()),
        checkpoint: CheckpointTrigger::ProcessExit,
        source: GameSource::Hdd,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
        matrix,
    }
}

fn declared(
    fw: &str,
    game_ver: Option<&str>,
    bench_max_steps: Option<u64>,
    checkpoint: Option<cellgov_boot::manifest::CheckpointTrigger>,
) -> MatrixCell {
    MatrixCell {
        key: CellKey {
            fw: fw.to_string(),
            game_ver: game_ver.map(str::to_string),
        },
        expect: CellExpectation::Frontier,
        bench_max_steps,
        checkpoint,
        pending: None,
    }
}

fn composition(firmware: FirmwareChoice, game: GameChoice) -> BootComposition {
    BootComposition {
        firmware,
        game,
        mounts: Vec::new(),
        eboot_dirs: Vec::new(),
        understated_firmware: Vec::new(),
        identity: cellgov_compare::RunIdentity::default(),
    }
}

fn managed(version: &str) -> FirmwareChoice {
    FirmwareChoice::Managed(ManagedFirmware {
        entry: FirmwareEntry {
            version: version.to_string(),
            entry_dir: PathBuf::from("store/firmware"),
            pup_sha256: "0".repeat(64),
            core_os: None,
        },
        selected_by: FirmwareSelectedBy::Flag,
    })
}

fn stored(version: GameVersion) -> GameChoice {
    GameChoice::Stored(Box::new(StoredGame {
        title_id: "CG_TEST".to_string(),
        version,
        base: BaseEntry {
            version: "01.00".to_string(),
            dir: PathBuf::from("store/titles/CG_TEST/base"),
            tree: cellgov_install::store::TitleTree::Game,
            distribution: "psn-hdd".to_string(),
            source_sha256: "0".repeat(64),
            system_ver: None,
            shipped_firmware: None,
        },
        update: None,
    }))
}

#[test]
fn a_stored_title_keys_on_the_firmware_and_the_selected_game_version() {
    let base = composed_cell(&composition(managed("4.93"), stored(GameVersion::Base)));
    assert_eq!(
        base,
        Some(CellKey {
            fw: "4.93".to_string(),
            game_ver: Some("base".to_string()),
        })
    );
    let update = composed_cell(&composition(
        managed("4.93"),
        stored(GameVersion::Update("02.51".to_string())),
    ));
    assert_eq!(
        update,
        Some(CellKey {
            fw: "4.93".to_string(),
            game_ver: Some("02.51".to_string()),
        })
    );
}

#[test]
fn a_firmware_shipped_title_keys_on_the_firmware_alone() {
    let cell = composed_cell(&composition(
        managed("4.93"),
        GameChoice::Firmware {
            dir: PathBuf::from("store/firmware/4.93/dev_flash/vsh/module"),
            unmanaged_path: false,
        },
    ));
    assert_eq!(
        cell,
        Some(CellKey {
            fw: "4.93".to_string(),
            game_ver: None,
        })
    );
}

#[test]
fn a_firmware_exec_path_outside_the_selected_entry_composes_no_cell() {
    let cell = composed_cell(&composition(
        managed("4.93"),
        GameChoice::Firmware {
            dir: PathBuf::from("/elsewhere/vsh/module"),
            unmanaged_path: true,
        },
    ));
    assert_eq!(cell, None);
}

#[test]
fn a_run_with_no_version_to_key_on_composes_no_cell() {
    let unmanaged = composition(
        FirmwareChoice::Unmanaged {
            dir: PathBuf::from("elsewhere/sys/external"),
        },
        stored(GameVersion::Base),
    );
    assert_eq!(composed_cell(&unmanaged), None);
    assert_eq!(
        composed_cell(&composition(
            FirmwareChoice::None,
            stored(GameVersion::Base)
        )),
        None
    );
    assert_eq!(
        composed_cell(&composition(managed("4.93"), GameChoice::Unstored)),
        None
    );
}

#[test]
fn a_declared_cells_overrides_are_what_the_run_and_the_anchor_are_taken_at() {
    use cellgov_boot::manifest::CheckpointTrigger;
    let title = manifest(
        Some(250_000_000),
        vec![declared(
            "4.93",
            Some("base"),
            Some(4_000),
            Some(CheckpointTrigger::FirstRsxWrite),
        )],
    );
    let plan = ResolvedPlan::resolve(
        &title,
        &composition(managed("4.93"), stored(GameVersion::Base)),
    );
    assert_eq!(plan.max_steps, 4_000);
    assert_eq!(plan.checkpoint, CheckpointTrigger::FirstRsxWrite);
    assert_eq!(plan.max_steps_usize(&title).expect("cap fits usize"), 4_000);
}

#[test]
fn an_undeclared_cell_takes_the_title_defaults() {
    use cellgov_boot::manifest::CheckpointTrigger;
    let title = manifest(
        Some(250_000_000),
        vec![declared("3.55", Some("base"), Some(4_000), None)],
    );
    let plan = ResolvedPlan::resolve(
        &title,
        &composition(managed("4.93"), stored(GameVersion::Base)),
    );
    assert_eq!(plan.max_steps, 250_000_000);
    assert_eq!(plan.checkpoint, CheckpointTrigger::ProcessExit);
}

#[test]
fn a_title_with_no_cap_anywhere_takes_the_recorder_default() {
    let title = manifest(None, Vec::new());
    let plan = ResolvedPlan::resolve(
        &title,
        &composition(managed("4.93"), stored(GameVersion::Base)),
    );
    assert_eq!(plan.max_steps, crate::paths::DEFAULT_BENCH_MAX_STEPS);
}
