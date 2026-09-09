use super::*;

use crate::game::manifest::{CellExpectation, Distribution, GameSource, MatrixCell};

fn cell(fw: &str, game_ver: Option<&str>, bench_max_steps: Option<u64>) -> MatrixCell {
    MatrixCell {
        key: CellKey {
            fw: fw.to_string(),
            game_ver: game_ver.map(str::to_string),
        },
        reference: false,
        expect: CellExpectation::Frontier,
        bench_max_steps,
        checkpoint: None,
        pending: None,
    }
}

fn manifest(matrix: Vec<MatrixCell>) -> TitleManifest {
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
        bench_max_steps: Some(4_000),
        checkpoint: CheckpointTrigger::ProcessExit,
        source: GameSource::Hdd,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
        matrix,
    }
}

#[test]
fn one_job_per_declared_cell_at_that_cells_cap() {
    let title = manifest(vec![
        cell("4.93", Some("base"), None),
        cell("3.55", Some("base"), Some(250_000)),
    ]);
    let jobs = jobs_for(&title);
    let caps: Vec<(String, u64)> = jobs.iter().map(|j| (j.cell.label(), j.max_steps)).collect();
    assert_eq!(
        caps,
        vec![
            ("fw 4.93 x base".to_string(), 4_000),
            ("fw 3.55 x base".to_string(), 250_000),
        ]
    );
}

#[test]
fn a_title_declaring_no_cells_yields_no_jobs() {
    assert!(jobs_for(&manifest(Vec::new())).is_empty());
}

#[test]
fn the_selection_narrows_to_the_named_cell() {
    let title = manifest(vec![
        cell("4.93", Some("base"), None),
        cell("3.55", Some("base"), None),
        cell("4.93", Some("01.02"), None),
    ]);
    let kept = filter_declared(jobs_for(&title), Some("4.93"), None);
    assert_eq!(
        kept.iter().map(Job::label).collect::<Vec<_>>(),
        vec!["test fw 4.93 x base", "test fw 4.93 x 01.02"]
    );

    let kept = filter_declared(jobs_for(&title), Some("4.93"), Some("01.02"));
    assert_eq!(
        kept.iter().map(Job::label).collect::<Vec<_>>(),
        vec!["test fw 4.93 x 01.02"]
    );
}

#[test]
fn no_selection_keeps_every_declared_cell() {
    let title = manifest(vec![
        cell("4.93", Some("base"), None),
        cell("3.55", Some("base"), None),
    ]);
    assert_eq!(filter_declared(jobs_for(&title), None, None).len(), 2);
}

fn key(fw: &str, game_ver: Option<&str>) -> CellKey {
    CellKey {
        fw: fw.to_string(),
        game_ver: game_ver.map(str::to_string),
    }
}

/// A `RUN_IDENTITY` payload naming one firmware and one game version.
fn identity(fw: Option<&str>, game_ver: Option<&str>) -> RunIdentity {
    RunIdentity {
        firmware: fw.map(|v| cellgov_compare::FirmwareIdentity {
            version: v.to_string(),
            image_version: "0000000000000000".to_string(),
            pup_sha256: "0".repeat(64),
        }),
        game: game_ver.map(|v| cellgov_compare::GameIdentity {
            title_id: "CG_TEST".to_string(),
            version: v.to_string(),
            app_version: Some(cellgov_compare::AppVersion::AppVer("01.00".to_string())),
        }),
    }
}

#[test]
fn an_identity_naming_the_cell_disagrees_about_nothing() {
    assert!(cell_disagreements(
        &identity(Some("4.93"), Some("base")),
        &key("4.93", Some("base"))
    )
    .is_empty());
    assert!(cell_disagreements(
        &identity(Some("4.93"), Some("update:02.51")),
        &key("4.93", Some("02.51"))
    )
    .is_empty());
    assert!(
        cell_disagreements(&identity(Some("4.93"), None), &key("4.93", None)).is_empty(),
        "a firmware-shipped title has no game-version axis to disagree about"
    );
}

#[test]
fn a_firmware_free_run_disagrees_with_the_cell_it_was_asked_for() {
    let found = cell_disagreements(&identity(None, Some("base")), &key("4.93", Some("base")));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("no managed firmware"), "{found:?}");
    assert!(found[0].contains("4.93"), "{found:?}");
}

#[test]
fn a_run_at_another_firmware_or_game_version_is_named_per_axis() {
    let found = cell_disagreements(
        &identity(Some("3.55"), Some("update:02.51")),
        &key("4.93", Some("base")),
    );
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found[0].contains("3.55") && found[0].contains("4.93"),
        "{found:?}"
    );
    assert!(
        found[1].contains("update:02.51") && found[1].contains("base"),
        "{found:?}"
    );
}

#[test]
fn an_unnamed_game_half_disagrees_with_a_cell_that_names_one() {
    let found = cell_disagreements(&identity(Some("4.93"), None), &key("4.93", Some("base")));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("(none)") && found[0].contains("base"),
        "{found:?}"
    );
}
