use super::super::test_fixtures::*;
use super::*;

#[test]
fn the_reference_is_the_reference_cells_load() {
    let fixtures = Fixtures::new("reference-derived");
    let t = title("NPAA62001", "Referenced", 2008, "Studio");
    let docs = load_title(&t, fixtures.path()).unwrap();
    assert_eq!(docs.reference().unwrap().key, reference_key());
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "does not declare the reference cell")]
fn a_reference_cell_a_game_title_omits_is_an_invariant_break_not_an_unrecorded_cell() {
    let fixtures = Fixtures::new("reference-omitted");
    let mut t = title("NPAA62002", "Omitted", 2008, "Studio");
    t.matrix = vec![matrix_cell(cell_key("3.55", Some(BASE)))];
    let docs = load_title(&t, fixtures.path()).unwrap();
    let _ = docs.reference();
}

#[test]
fn a_firmware_shipped_title_without_a_reference_row_has_no_loaded_reference() {
    let fixtures = Fixtures::new("reference-firmware-row");
    let t = firmware_exec_title("VSHREF", "System Software", &["3.70"]);
    let docs = load_title(&t, fixtures.path()).unwrap();
    assert!(docs.reference().is_none());
    let t = firmware_exec_title("VSHREF", "System Software", &["3.70", REFERENCE_FW]);
    let docs = load_title(&t, fixtures.path()).unwrap();
    assert_eq!(
        docs.reference().unwrap().key,
        cell_key(REFERENCE_FW, None),
        "its reference is its row at the reference firmware"
    );
}
