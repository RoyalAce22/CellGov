//! The output set: which files the generator owns, and what happens to
//! a page no title claims.

use super::super::test_fixtures::*;
use super::*;

fn paths(docs: &[GeneratedDoc]) -> Vec<String> {
    docs.iter()
        .map(|d| d.path.to_string_lossy().replace('\\', "/"))
        .collect()
}

#[test]
fn the_output_set_is_the_index_plus_one_page_per_title() {
    let a = title("NPAA90001", "A", 2009, "Studio");
    let b = title("NPAA90002", "B", 2009, "Studio");
    let fixtures = Fixtures::new("output-set");
    let docs = render_docs([&a, &b], fixtures.path()).unwrap();
    assert_eq!(
        paths(&docs),
        ["titles.md", "titles/NPAA90001.md", "titles/NPAA90002.md"]
    );
}

#[test]
fn the_pages_follow_content_id_order_whatever_order_the_registry_yields() {
    let a = title("NPAA90003", "Three", 2009, "Studio");
    let b = title("NPAA90001", "One", 2009, "Studio");
    let c = title("NPAA90002", "Two", 2009, "Studio");
    let fixtures = Fixtures::new("output-order");
    let docs = render_docs([&a, &b, &c], fixtures.path()).unwrap();
    assert_eq!(
        paths(&docs),
        [
            "titles.md",
            "titles/NPAA90001.md",
            "titles/NPAA90002.md",
            "titles/NPAA90003.md"
        ]
    );
}

#[test]
fn an_empty_registry_still_emits_the_index_alone() {
    let fixtures = Fixtures::new("output-empty");
    let docs = render_docs(std::iter::empty(), fixtures.path()).unwrap();
    assert_eq!(paths(&docs), ["titles.md"]);
}

#[test]
fn a_page_no_title_claims_is_an_orphan() {
    let t = title("NPAA90004", "Kept", 2009, "Studio");
    let fixtures = Fixtures::new("orphan");
    let out = cellgov_testkit::scratch::scratch_labeled("orphan-out");
    let docs = render_docs([&t], fixtures.path()).unwrap();

    let detail_dir = out.join("titles");
    std::fs::create_dir_all(&detail_dir).unwrap();
    std::fs::write(detail_dir.join("NPAA90004.md"), "kept").unwrap();
    std::fs::write(detail_dir.join("NPAA99999.md"), "dropped").unwrap();
    std::fs::write(detail_dir.join("NOTES.txt"), "not a page").unwrap();

    let orphans = orphaned_pages(&out, &docs).unwrap();
    assert_eq!(orphans, vec![detail_dir.join("NPAA99999.md")]);
}

#[test]
fn an_output_directory_with_no_pages_yet_has_no_orphans() {
    let t = title("NPAA90005", "Fresh", 2009, "Studio");
    let fixtures = Fixtures::new("orphan-fresh");
    let out = cellgov_testkit::scratch::scratch_labeled("orphan-fresh-out");
    let docs = render_docs([&t], fixtures.path()).unwrap();
    assert!(orphaned_pages(&out, &docs).unwrap().is_empty());
}

#[test]
fn a_load_failure_stops_the_whole_set_rather_than_emitting_part_of_it() {
    let mut t = title("NPAA90006", "Undeclared", 2009, "Studio");
    t.matrix.clear();
    let fixtures = Fixtures::new("output-refused");
    fixtures.write_cross("NPAA90006", &reference_key(), &converged(0));
    match render_docs([&t], fixtures.path()) {
        Err(SummaryLoadError::UndeclaredCell {
            cell, content_id, ..
        }) => {
            assert_eq!(cell, "fw 4.93 x base");
            assert_eq!(content_id, "NPAA90006");
        }
        Err(other) => panic!("expected an undeclared-cell refusal, got {other:?}"),
        Ok(docs) => panic!("expected a refusal, got {} document(s)", docs.len()),
    }
}
