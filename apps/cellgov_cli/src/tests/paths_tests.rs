use std::path::Path;

use super::*;
use cellgov_boot::manifest::{CellExpectation, CheckpointTrigger, MatrixCell};

fn key(fw: &str, game_ver: Option<&str>) -> CellKey {
    CellKey {
        fw: fw.to_string(),
        game_ver: game_ver.map(str::to_string),
    }
}

fn cell(bench_max_steps: Option<u64>, checkpoint: Option<CheckpointTrigger>) -> MatrixCell {
    MatrixCell {
        key: key("4.93", Some("base")),
        expect: CellExpectation::Frontier,
        bench_max_steps,
        checkpoint,
        pending: None,
    }
}

fn manifest() -> TitleManifest {
    use cellgov_boot::manifest::{Distribution, GameSource};
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
fn each_cell_of_a_title_gets_its_own_anchor_file() {
    let root = Path::new("/w");
    let a = boot_anchor_path(root, "NPUA80001", &key("4.93", Some("base")));
    let b = boot_anchor_path(root, "NPUA80001", &key("3.55", Some("base")));
    let c = boot_anchor_path(root, "NPUA80001", &key("4.93", Some("01.02")));
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_ne!(b, c);
    assert_ne!(
        a,
        history_path(root, "NPUA80001", &key("4.93", Some("base")))
    );
}

#[test]
fn a_cell_path_names_its_firmware_and_game_version() {
    let path = boot_anchor_path(Path::new("/w"), "NPUA80001", &key("4.93", Some("base")));
    let rendered = path.to_string_lossy().replace('\\', "/");
    assert!(
        rendered
            .ends_with("tests/fixtures/NPUA80001/cellgov/anchors/fw-4.93/base/boot_summary.json"),
        "{rendered}"
    );
}

#[test]
fn a_cell_with_no_game_version_axis_stops_at_the_firmware_segment() {
    let path = boot_anchor_path(Path::new("/w"), "VSH", &key("4.93", None));
    let rendered = path.to_string_lossy().replace('\\', "/");
    assert!(
        rendered.ends_with("tests/fixtures/VSH/cellgov/anchors/fw-4.93/boot_summary.json"),
        "{rendered}"
    );
}

#[test]
fn a_cell_override_wins_over_the_title_default_and_the_recorder_default() {
    let title = manifest();
    assert_eq!(cell_max_steps(&title, None), 7_000);
    assert_eq!(cell_max_steps(&title, Some(&cell(Some(250), None))), 250);
    assert_eq!(
        title.cell_checkpoint(Some(&cell(None, Some(CheckpointTrigger::FirstRsxWrite)))),
        CheckpointTrigger::FirstRsxWrite
    );
    assert_eq!(title.cell_checkpoint(None), CheckpointTrigger::ProcessExit);

    let mut bare = manifest();
    bare.bench_max_steps = None;
    assert_eq!(cell_max_steps(&bare, None), DEFAULT_BENCH_MAX_STEPS);
}

#[test]
fn a_cell_declaring_no_override_falls_back_to_the_title() {
    let title = manifest();
    let plain = cell(None, None);
    assert_eq!(cell_max_steps(&title, Some(&plain)), 7_000);
    assert_eq!(
        title.cell_checkpoint(Some(&plain)),
        CheckpointTrigger::ProcessExit
    );

    let mut bare = manifest();
    bare.bench_max_steps = None;
    assert_eq!(
        cell_max_steps(&bare, Some(&plain)),
        DEFAULT_BENCH_MAX_STEPS,
        "neither the cell nor the title declares a cap"
    );
}
