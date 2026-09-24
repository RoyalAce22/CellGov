use std::path::Path;

use super::*;

fn key(fw: &str, game_ver: Option<&str>) -> CellKey {
    CellKey {
        fw: fw.to_string(),
        game_ver: game_ver.map(str::to_string),
    }
}

#[test]
fn a_workspace_roots_anchor_sits_in_its_fixture_tree_beside_the_cells_history() {
    let cell = key("4.93", Some("base"));
    let anchor = boot_anchor_path(Path::new("/w"), "NPUA80001", &cell);
    let rendered = anchor.to_string_lossy().replace('\\', "/");
    assert!(
        rendered
            .ends_with("tests/fixtures/NPUA80001/cellgov/anchors/fw-4.93/base/boot_summary.json"),
        "{rendered}"
    );
    let history = history_path(Path::new("/w"), "NPUA80001", &cell);
    assert_ne!(anchor, history);
    assert_eq!(anchor.parent(), history.parent());
}
