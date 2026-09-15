use super::*;

use cellgov_boot::manifest::{CellExpectation, Distribution, GameSource, MatrixCell};

fn cell(fw: &str, game_ver: Option<&str>, bench_max_steps: Option<u64>) -> MatrixCell {
    MatrixCell {
        key: CellKey {
            fw: fw.to_string(),
            game_ver: game_ver.map(str::to_string),
        },
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

#[test]
fn one_cell_per_declared_row_at_that_rows_cap() {
    let title = manifest(vec![
        cell("4.93", Some("base"), None),
        cell("3.55", Some("base"), Some(250_000)),
    ]);
    let cells = declared_cells(&title);
    let caps: Vec<(String, u64)> = cells
        .iter()
        .map(|c| (c.cell.label(), c.max_steps))
        .collect();
    assert_eq!(
        caps,
        vec![
            ("fw 4.93 x base".to_string(), 4_000),
            ("fw 3.55 x base".to_string(), 250_000),
        ]
    );
}

#[test]
fn a_title_declaring_no_cells_yields_none() {
    assert!(declared_cells(&manifest(Vec::new())).is_empty());
}

#[test]
fn the_selection_narrows_to_the_named_cell() {
    let title = manifest(vec![
        cell("4.93", Some("base"), None),
        cell("3.55", Some("base"), None),
        cell("4.93", Some("01.02"), None),
    ]);
    let kept = filter_declared(declared_cells(&title), Some("4.93"), None, "t");
    assert_eq!(
        kept.iter().map(DeclaredCell::label).collect::<Vec<_>>(),
        vec!["test fw 4.93 x base", "test fw 4.93 x 01.02"]
    );

    let kept = filter_declared(declared_cells(&title), Some("4.93"), Some("01.02"), "t");
    assert_eq!(
        kept.iter().map(DeclaredCell::label).collect::<Vec<_>>(),
        vec!["test fw 4.93 x 01.02"]
    );
}

#[test]
fn no_selection_keeps_every_declared_cell() {
    let title = manifest(vec![
        cell("4.93", Some("base"), None),
        cell("3.55", Some("base"), None),
    ]);
    assert_eq!(
        filter_declared(declared_cells(&title), None, None, "t").len(),
        2
    );
}

#[test]
fn a_pending_cell_is_set_aside_unless_asked_for() {
    let mut stopped = cell("1.50", Some("base"), None);
    stopped.pending = Some("the firmware cannot be obtained".to_string());
    let title = manifest(vec![stopped, cell("4.93", Some("base"), None)]);

    let (kept, aside) = split_pending(declared_cells(&title), false);
    assert_eq!(
        kept.iter().map(DeclaredCell::label).collect::<Vec<_>>(),
        vec!["test fw 4.93 x base"]
    );
    assert_eq!(
        aside.iter().map(DeclaredCell::label).collect::<Vec<_>>(),
        vec!["test fw 1.50 x base"]
    );
    assert_eq!(
        aside[0].pending.as_deref(),
        Some("the firmware cannot be obtained")
    );

    let (kept, aside) = split_pending(declared_cells(&title), true);
    assert_eq!(kept.len(), 2, "asked for, the cell is kept");
    assert!(aside.is_empty());
}

#[test]
fn titles_read_from_a_registry_come_back_in_short_name_order() {
    let dir = cellgov_testkit::scratch::scratch_labeled("declared_cells_registry");
    for (id, name) in [("CG_B", "zeta"), ("CG_A", "alpha")] {
        std::fs::write(
            dir.join(format!("{id}.toml")),
            format!(
                "[title]\ncontent_id = \"{id}\"\nshort_name = \"{name}\"\n\
                 display_name = \"{name}\"\neboot_candidates = [\"EBOOT.BIN\"]\n\
                 year = 2007\ndeveloper = \"d\"\nengine = \"e\"\n\
                 distribution = \"psn-hdd\"\nsystem_ver = \"4.93\"\n\n\
                 [checkpoint]\nkind = \"process-exit\"\n"
            ),
        )
        .expect("write manifest");
    }
    let titles = read_registry(&dir);
    let names: Vec<&str> = titles.iter().map(|t| t.short_name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "zeta"]);
    assert_eq!(select_titles(&titles, None).len(), 2);
    assert_eq!(select_titles(&titles, Some("zeta"))[0].content_id, "CG_B");
}
