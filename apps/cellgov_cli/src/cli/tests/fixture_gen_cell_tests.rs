//! Where a cell's fixture triple lands, and where the generated
//! reproduction's links resolve from there.

use super::*;
use crate::game::manifest::CellKey;

fn cell(fw: &str, game_ver: Option<&str>) -> CellKey {
    CellKey {
        fw: fw.to_string(),
        game_ver: game_ver.map(str::to_string),
    }
}

/// Path components below the workspace root, in the committed fixture
/// tree.
fn committed_depth(cell: &CellKey) -> usize {
    crate::paths::cell_cross_runner_dir_in(
        Path::new(crate::paths::DEFAULT_FIXTURES_DIR),
        "NPUA80001",
        cell,
    )
    .components()
    .count()
}

#[test]
fn the_reproduction_climbs_exactly_as_far_as_its_cell_is_deep() {
    for c in [cell("4.93", Some("base")), cell("4.93", None)] {
        let (repo_root, observations) = relative_prefixes(&c);
        assert_eq!(repo_root.matches("../").count(), committed_depth(&c));
        assert_eq!(
            observations.matches("../").count(),
            committed_depth(&c) - CONTENT_ID_DEPTH
        );
    }
}

#[test]
fn a_firmware_shipped_cell_sits_one_level_shallower() {
    let (with_game_ver, _) = relative_prefixes(&cell("4.93", Some("base")));
    let (firmware_exec, _) = relative_prefixes(&cell("4.93", None));
    assert_eq!(with_game_ver.len(), firmware_exec.len() + "../".len());
}

#[test]
fn the_reproduced_flags_name_the_cell_the_triple_is_filed_under() {
    assert_eq!(
        selection_flags(&cell("4.93", Some("base"))),
        "--fw 4.93 --game-ver base"
    );
    assert_eq!(
        selection_flags(&cell("4.93", Some("02.51"))),
        "--fw 4.93 --game-ver 02.51"
    );
    assert_eq!(selection_flags(&cell("4.93", None)), "--fw 4.93");
}

/// Resolve a `../` chain against `dir`, the way a reader resolves a
/// relative link out of that directory.
fn climb(dir: &Path, prefix: &str) -> PathBuf {
    let mut out = dir.to_path_buf();
    for _ in 0..prefix.matches("../").count() {
        assert!(out.pop(), "climbed past the root of {}", dir.display());
    }
    out
}

#[test]
fn the_observation_link_lands_on_the_titles_own_directory() {
    for c in [cell("4.93", Some("base")), cell("4.93", None)] {
        let dir = crate::paths::cell_cross_runner_dir_in(
            Path::new(crate::paths::DEFAULT_FIXTURES_DIR),
            "NPUA80001",
            &c,
        );
        let (_, observations) = relative_prefixes(&c);
        assert_eq!(
            climb(&dir, &observations),
            Path::new(crate::paths::DEFAULT_FIXTURES_DIR).join("NPUA80001"),
        );
    }
}

#[test]
fn the_workspace_root_link_lands_on_the_workspace_root() {
    for c in [cell("4.93", Some("base")), cell("4.93", None)] {
        let dir = crate::paths::cell_cross_runner_dir_in(
            Path::new(crate::paths::DEFAULT_FIXTURES_DIR),
            "NPUA80001",
            &c,
        );
        let (repo_root, _) = relative_prefixes(&c);
        assert_eq!(climb(&dir, &repo_root), Path::new(""));
    }
}

#[test]
fn the_reproduction_leaves_no_template_token_unfilled() {
    for c in [cell("4.93", Some("base")), cell("4.93", None)] {
        let body = reproduction_body("NPUA80001", "flOw", "process-exit", &c);
        assert!(
            !body.contains("{{"),
            "an unfilled template key remains:\n{body}"
        );
    }
}

#[test]
fn the_reproduction_names_the_cell_it_was_filed_under() {
    let c = cell("2.76", Some("base"));
    let body = reproduction_body("BCES00664", "WipEout HD", "process-exit", &c);
    assert!(body.contains("fw 2.76 x base"), "{body}");
    assert!(body.contains("--fw 2.76 --game-ver base"), "{body}");
    assert!(body.contains("fw-2.76/base"), "{body}");
}

#[test]
fn a_fixture_tree_outside_the_committed_one_is_not_taken_for_it() {
    assert!(is_committed_tree(Path::new(
        crate::paths::DEFAULT_FIXTURES_DIR
    )));
    assert!(is_committed_tree(&crate::paths::fixtures_dir(
        &crate::paths::workspace_root()
    )));
    assert!(!is_committed_tree(Path::new("out")));
    assert!(!is_committed_tree(Path::new("tests/fixtures/elsewhere")));
}

#[test]
fn two_cells_of_one_title_do_not_share_a_directory() {
    let fixtures = Path::new("tests/fixtures");
    let a =
        crate::paths::cell_cross_runner_dir_in(fixtures, "NPUA80001", &cell("4.93", Some("base")));
    let b =
        crate::paths::cell_cross_runner_dir_in(fixtures, "NPUA80001", &cell("3.55", Some("base")));
    let c =
        crate::paths::cell_cross_runner_dir_in(fixtures, "NPUA80001", &cell("4.93", Some("02.51")));
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_ne!(b, c);
}
