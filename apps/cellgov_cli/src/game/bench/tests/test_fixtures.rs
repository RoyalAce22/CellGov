//! Fixtures shared by more than one of the `bench` submodule test
//! files.

use std::time::Duration;

use cellgov_compare::bench::{BenchBootResult, MeasuredRun};
use cellgov_compare::BootOutcome;
use cellgov_time::Budget;

use super::options::{AnchorPlan, BenchOptions, SelectionArgs};
use super::throughput::ThroughputPolicy;
use cellgov_boot::manifest::{self, CellKey};

/// A run of `wall` that agrees with every other run this helper
/// builds, so a case that varies only the wall isolates the throughput
/// half.
pub(super) fn run_of(run_index: usize, wall: Duration) -> BenchBootResult {
    BenchBootResult {
        run_index,
        steps: 10,
        wall,
        budget: Budget::new(256),
        outcome: BootOutcome::ProcessExit,
    }
}

/// A set whose walls are `walls`, indexed in order.
pub(super) fn set_of(walls: &[Duration]) -> Vec<BenchBootResult> {
    walls
        .iter()
        .enumerate()
        .map(|(i, w)| run_of(i, *w))
        .collect()
}

pub(super) fn reporting() -> ThroughputPolicy {
    ThroughputPolicy {
        runs: 3,
        strict: false,
    }
}

pub(super) fn strict() -> ThroughputPolicy {
    ThroughputPolicy {
        runs: 3,
        strict: true,
    }
}

/// A run of the anchor check's shape, over `stderr`.
pub(super) fn measured_run(stderr: &str) -> MeasuredRun<'_> {
    MeasuredRun {
        checkpoint: manifest::CheckpointTrigger::ProcessExit.kind(),
        steps: 1,
        budget: Budget::new(256),
        outcome: BootOutcome::MaxSteps,
        stderr,
    }
}

/// A cell of the committed fixture tree's shape.
pub(super) fn test_cell() -> CellKey {
    CellKey {
        fw: "4.93".to_string(),
        game_ver: Some("base".to_string()),
    }
}

pub(super) fn bench_manifest(
    bench_max_steps: Option<u64>,
) -> cellgov_boot::manifest::TitleManifest {
    use cellgov_boot::manifest::{Distribution, GameSource};
    cellgov_boot::manifest::TitleManifest {
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
        checkpoint: manifest::CheckpointTrigger::ProcessExit,
        source: GameSource::Hdd,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
        matrix: Vec::new(),
    }
}

/// A run of `title` in `cell`, at exactly what the registry declares
/// for it.
pub(super) fn bench_options<'a>(
    title: &'a cellgov_boot::manifest::TitleManifest,
    cell: Option<&'a CellKey>,
    guest_args: &'a [String],
) -> BenchOptions<'a> {
    let max_steps = title.cell_max_steps(None);
    BenchOptions {
        title,
        elf_path: "EBOOT.BIN",
        max_steps: max_steps as usize,
        plan: AnchorPlan {
            cell,
            max_steps,
            checkpoint: title.checkpoint_trigger(),
        },
        firmware_dir: None,
        composed_mounts: &[],
        eboot_dirs: &[],
        identity: &cellgov_compare::RunIdentity {
            firmware: None,
            game: None,
            overrides: cellgov_compare::BootOverrides {
                skip_module_start: false,
                force_system_authid: false,
                prx_base: None,
                disable_module_start_hle_stubs: false,
            },
        },
        selection: SelectionArgs::default(),
        strict_reserved: false,
        checkpoint_override: None,
        budget_override: None,
        prescan: false,
        guest_args,
        check_anchor: true,
        run_index: 0,
    }
}
