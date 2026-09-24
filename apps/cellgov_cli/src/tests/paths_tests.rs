use std::path::Path;

use super::*;

fn key(fw: &str, game_ver: Option<&str>) -> CellKey {
    CellKey {
        fw: fw.to_string(),
        game_ver: game_ver.map(str::to_string),
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
