//! A cell's cap and checkpoint fall back to its title's, then to the
//! recorder's default.

use super::*;
use crate::manifest::{CellExpectation, CellKey, MatrixCell};

fn cell(bench_max_steps: Option<u64>, checkpoint: Option<CheckpointTrigger>) -> MatrixCell {
    MatrixCell {
        key: CellKey {
            fw: "4.93".to_string(),
            game_ver: Some("base".to_string()),
        },
        expect: CellExpectation::Frontier,
        bench_max_steps,
        checkpoint,
        pending: None,
    }
}

fn manifest() -> TitleManifest {
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
        bench_max_steps: Some(7_000),
        system_ver: Some("4.93".to_string()),
        checkpoint: CheckpointTrigger::ProcessExit,
        source: GameSource::Hdd,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
        matrix: Vec::new(),
    }
}

#[test]
fn a_cell_override_wins_over_the_title_default_and_the_recorder_default() {
    let title = manifest();
    assert_eq!(title.cell_max_steps(None), 7_000);
    assert_eq!(title.cell_max_steps(Some(&cell(Some(250), None))), 250);
    assert_eq!(
        title.cell_checkpoint(Some(&cell(None, Some(CheckpointTrigger::FirstRsxWrite)))),
        CheckpointTrigger::FirstRsxWrite
    );
    assert_eq!(title.cell_checkpoint(None), CheckpointTrigger::ProcessExit);

    let mut bare = manifest();
    bare.bench_max_steps = None;
    assert_eq!(bare.cell_max_steps(None), DEFAULT_BENCH_MAX_STEPS);
}

#[test]
fn a_cell_declaring_no_override_falls_back_to_the_title() {
    let title = manifest();
    let plain = cell(None, None);
    assert_eq!(title.cell_max_steps(Some(&plain)), 7_000);
    assert_eq!(
        title.cell_checkpoint(Some(&plain)),
        CheckpointTrigger::ProcessExit
    );

    let mut bare = manifest();
    bare.bench_max_steps = None;
    assert_eq!(
        bare.cell_max_steps(Some(&plain)),
        DEFAULT_BENCH_MAX_STEPS,
        "neither the cell nor the title declares a cap"
    );
}
