//! The per-title grid: its axes, its blanks, and its reference mark.

use cellgov_compare::BootOutcome;

use super::super::load::load_title;
use super::super::test_fixtures::*;
use super::*;
use crate::game::manifest::TitleManifest;

/// The first entry of `cells` becomes the title's reference cell.
fn title_with(content_id: &str, cells: &[(&str, Option<&str>)]) -> TitleManifest {
    let mut t = title(content_id, "Gridded", 2009, "Studio");
    t.matrix = cells
        .iter()
        .enumerate()
        .map(|(i, (fw, game))| matrix_cell(cell_key(fw, *game), i == 0))
        .collect();
    t
}

fn grid_of(t: &TitleManifest, fixtures: &std::path::Path) -> String {
    render_grid(&load_title(t, fixtures).unwrap())
}

#[test]
fn firmware_runs_down_the_side_and_game_version_across() {
    let t = title_with(
        "NPAA80001",
        &[
            (REFERENCE_FW, Some(BASE)),
            (REFERENCE_FW, Some("02.51")),
            ("3.55", Some(BASE)),
        ],
    );
    let fixtures = Fixtures::new("grid-axes");
    let grid = grid_of(&t, fixtures.path());
    assert!(grid.starts_with("| fw \\ game | base | 02.51 |"), "{grid}");
    assert!(grid.contains("\n| 3.55 |"), "{grid}");
    assert!(grid.contains("\n| 4.93 |"), "{grid}");
}

#[test]
fn base_leads_the_game_axis_whatever_it_sorts_against() {
    let t = title_with(
        "NPAA80002",
        &[
            (REFERENCE_FW, Some(BASE)),
            (REFERENCE_FW, Some("01.02")),
            (REFERENCE_FW, Some("02.51")),
        ],
    );
    let fixtures = Fixtures::new("grid-base-first");
    let grid = grid_of(&t, fixtures.path());
    assert!(
        grid.starts_with("| fw \\ game | base | 01.02 | 02.51 |"),
        "{grid}"
    );
}

#[test]
fn an_undeclared_intersection_renders_blank_not_a_dot() {
    let t = title_with(
        "NPAA80003",
        &[(REFERENCE_FW, Some(BASE)), ("3.55", Some("02.51"))],
    );
    let fixtures = Fixtures::new("grid-blank");
    let grid = grid_of(&t, fixtures.path());
    // 4.93 declares only `base`, so its `02.51` intersection is out of
    // scope. Its `base` cell is declared but not measured.
    assert!(grid.contains("| 4.93 | .* |  |"), "{grid}");
    assert!(grid.contains("| 3.55 |  | . |"), "{grid}");
}

#[test]
fn the_reference_cell_carries_the_star_and_no_other_cell_does() {
    let t = title_with(
        "NPAA80004",
        &[(REFERENCE_FW, Some(BASE)), ("3.55", Some(BASE))],
    );
    let fixtures = Fixtures::new("grid-star");
    fixtures.write_cross("NPAA80004", &reference_key(), &converged(0));
    fixtures.write_cross("NPAA80004", &cell_key("3.55", Some(BASE)), &converged(0));
    let grid = grid_of(&t, fixtures.path());
    assert!(grid.contains("| 4.93 | ok* |"), "{grid}");
    assert!(grid.contains("| 3.55 | ok |"), "{grid}");
    assert_eq!(grid.matches('*').count(), 1, "{grid}");
}

#[test]
fn a_firmware_shipped_title_has_no_game_axis() {
    let t = title_with("VSHTEST", &[(REFERENCE_FW, None), ("4.91", None)]);
    let fixtures = Fixtures::new("grid-no-game-axis");
    fixtures.write_anchor(
        "VSHTEST",
        &cell_key(REFERENCE_FW, None),
        &boot(BootOutcome::MaxSteps, 389_859),
    );
    let grid = grid_of(&t, fixtures.path());
    assert!(grid.starts_with("| fw | result |"), "{grid}");
    assert!(grid.contains("| 4.93 | anchor (MaxSteps)* |"), "{grid}");
    assert!(grid.contains("| 4.91 | . |"), "{grid}");
}

#[test]
fn a_title_declaring_no_cells_says_so_instead_of_an_empty_table() {
    let mut t = title("NPAA80005", "NoCells", 2009, "Studio");
    t.matrix.clear();
    let fixtures = Fixtures::new("grid-none");
    let grid = grid_of(&t, fixtures.path());
    assert_eq!(grid, "This title declares no cells.");
    assert!(!grid.contains('|'), "{grid}");
}

#[test]
fn the_page_names_the_title_and_links_back_to_the_index() {
    let t = title_with("NPAA80006", &[(REFERENCE_FW, Some(BASE))]);
    let fixtures = Fixtures::new("page-header");
    let page = render(&load_title(&t, fixtures.path()).unwrap());
    assert!(page.starts_with("# Gridded (NPAA80006)"), "{page}");
    assert!(page.contains("[matrix](../titles.md)"), "{page}");
    assert!(!page.contains("{{"), "unsubstituted token in {page}");
}

#[test]
fn the_page_is_byte_identical_across_two_renders() {
    let t = title_with(
        "NPAA80007",
        &[(REFERENCE_FW, Some(BASE)), ("3.55", Some(BASE))],
    );
    let fixtures = Fixtures::new("page-deterministic");
    fixtures.write_cross("NPAA80007", &reference_key(), &converged(3));
    let docs = load_title(&t, fixtures.path()).unwrap();
    assert_eq!(render(&docs), render(&docs));
}

#[test]
fn detail_page_path_and_link_name_the_same_file() {
    assert_eq!(
        detail_page_path("NPAA00002"),
        std::path::Path::new("titles").join("NPAA00002.md")
    );
    assert_eq!(
        detail_page_link("NPAA00002"),
        "[NPAA00002](titles/NPAA00002.md)"
    );
}
