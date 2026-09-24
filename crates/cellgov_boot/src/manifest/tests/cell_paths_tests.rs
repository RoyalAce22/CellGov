//! The per-cell layout of the fixture tree.

use std::path::Path;

use super::*;

fn key(fw: &str, game_ver: Option<&str>) -> CellKey {
    CellKey {
        fw: fw.to_string(),
        game_ver: game_ver.map(str::to_string),
    }
}

fn rendered(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn each_cell_of_a_title_gets_its_own_anchor_file() {
    let fixtures = Path::new("f");
    let a = boot_anchor_path_in(fixtures, "NPUA80001", &key("4.93", Some("base")));
    let b = boot_anchor_path_in(fixtures, "NPUA80001", &key("3.55", Some("base")));
    let c = boot_anchor_path_in(fixtures, "NPUA80001", &key("4.93", Some("01.02")));
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_ne!(b, c);
}

#[test]
fn a_cell_path_names_its_firmware_and_game_version() {
    let path = boot_anchor_path_in(Path::new("f"), "NPUA80001", &key("4.93", Some("base")));
    assert_eq!(
        rendered(&path),
        "f/NPUA80001/cellgov/anchors/fw-4.93/base/boot_summary.json"
    );
}

#[test]
fn a_cell_with_no_game_version_axis_stops_at_the_firmware_segment() {
    let path = boot_anchor_path_in(Path::new("f"), "VSH", &key("4.93", None));
    assert_eq!(
        rendered(&path),
        "f/VSH/cellgov/anchors/fw-4.93/boot_summary.json"
    );
}

#[test]
fn a_cells_cross_runner_summary_sits_under_the_titles_cross_runner_tree() {
    let cell = key("4.93", Some("base"));
    let path = cross_runner_summary_path_in(Path::new("f"), "NPUA80001", &cell);
    assert_eq!(
        rendered(&path),
        "f/NPUA80001/cross_runner/fw-4.93/base/cross_runner_summary.json"
    );
    assert!(path.starts_with(title_cross_runner_dir_in(Path::new("f"), "NPUA80001")));
    assert!(boot_anchor_path_in(Path::new("f"), "NPUA80001", &cell)
        .starts_with(title_anchors_dir_in(Path::new("f"), "NPUA80001")));
}
