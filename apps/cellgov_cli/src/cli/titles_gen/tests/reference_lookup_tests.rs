use super::super::test_fixtures::*;
use super::*;

#[test]
fn the_reference_is_the_derived_cells_load() {
    let fixtures = Fixtures::new("reference-derived");
    let t = title("NPAA62001", "Derived", 2008, "Studio");
    let docs = load_title(&t, fixtures.path()).unwrap();
    assert_eq!(docs.reference().unwrap().key, reference_key());
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "does not declare it")]
fn a_derived_cell_the_matrix_omits_is_an_invariant_break_not_an_unrecorded_cell() {
    let fixtures = Fixtures::new("reference-omitted");
    let mut t = title("NPAA62002", "Omitted", 2008, "Studio");
    t.matrix = vec![matrix_cell(cell_key("3.55", Some(BASE)))];
    let docs = load_title(&t, fixtures.path()).unwrap();
    let _ = docs.reference();
}
