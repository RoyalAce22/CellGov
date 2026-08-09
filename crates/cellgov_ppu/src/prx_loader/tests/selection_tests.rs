use std::collections::{BTreeMap, BTreeSet};

use super::{select_import_closure, ClosureSelection};
use crate::sprx::test_fixtures::make_test_prx_graph_node;

fn candidates(items: &[(&str, Vec<u8>)]) -> BTreeMap<String, Vec<u8>> {
    items
        .iter()
        .map(|(p, b)| ((*p).to_string(), b.clone()))
        .collect()
}

fn roots(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| (*s).to_string()).collect()
}

fn selected(sel: &ClosureSelection) -> Vec<&str> {
    sel.selected.iter().map(String::as_str).collect()
}

#[test]
fn a_root_namespace_pulls_its_provider_and_the_provider_deps() {
    // b exports libbbbb and imports libaaaa; a provides libaaaa.
    let cands = candidates(&[
        (
            "a.sprx",
            make_test_prx_graph_node("modaaaa", "libaaaa", None),
        ),
        (
            "b.sprx",
            make_test_prx_graph_node("modbbbb", "libbbbb", Some("libaaaa")),
        ),
    ]);
    let sel = select_import_closure(&cands, Some(&roots(&["libbbbb"]))).expect("select");
    assert_eq!(selected(&sel), vec!["a.sprx", "b.sprx"]);
    assert!(sel.pruned.is_empty());
    assert!(sel.unprovided_roots.is_empty());
}

#[test]
fn an_unimported_module_stays_out_of_the_selection() {
    let cands = candidates(&[
        (
            "a.sprx",
            make_test_prx_graph_node("modaaaa", "libaaaa", None),
        ),
        (
            "b.sprx",
            make_test_prx_graph_node("modbbbb", "libbbbb", None),
        ),
    ]);
    let sel = select_import_closure(&cands, Some(&roots(&["libaaaa"]))).expect("select");
    assert_eq!(selected(&sel), vec!["a.sprx"]);
}

#[test]
fn a_root_with_no_provider_is_reported_not_fatal() {
    let cands = candidates(&[(
        "a.sprx",
        make_test_prx_graph_node("modaaaa", "libaaaa", None),
    )]);
    let sel = select_import_closure(&cands, Some(&roots(&["ghostns"]))).expect("select");
    assert!(sel.selected.is_empty());
    assert_eq!(
        sel.unprovided_roots,
        roots(&["ghostns"]),
        "the caller's unresolved-import handling covers these NIDs"
    );
}

#[test]
fn a_module_with_an_unsatisfiable_import_is_pruned_with_the_namespace_named() {
    let cands = candidates(&[
        (
            "a.sprx",
            make_test_prx_graph_node("modaaaa", "libaaaa", None),
        ),
        (
            "c.sprx",
            make_test_prx_graph_node("modcccc", "libcccc", Some("ghostns")),
        ),
    ]);
    let sel = select_import_closure(&cands, None).expect("select");
    assert_eq!(selected(&sel), vec!["a.sprx"]);
    assert_eq!(
        sel.pruned,
        vec![(
            "c.sprx".to_string(),
            super::PruneReason::UnprovidedImport("ghostns".to_string())
        )]
    );
}

#[test]
fn pruning_cascades_through_dependents() {
    // c is unsatisfiable; b needs c's library, so b prunes too.
    let cands = candidates(&[
        (
            "b.sprx",
            make_test_prx_graph_node("modbbbb", "libbbbb", Some("libcccc")),
        ),
        (
            "c.sprx",
            make_test_prx_graph_node("modcccc", "libcccc", Some("ghostns")),
        ),
    ]);
    let sel = select_import_closure(&cands, None).expect("select");
    assert!(sel.selected.is_empty());
    let pruned_paths: BTreeSet<&str> = sel.pruned.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(pruned_paths, ["b.sprx", "c.sprx"].into_iter().collect());
}

#[test]
fn a_permitted_missing_namespace_does_not_prune() {
    let cands = candidates(&[(
        "a.sprx",
        make_test_prx_graph_node("modaaaa", "libaaaa", Some("cellLibprof")),
    )]);
    let sel = select_import_closure(&cands, None).expect("select");
    assert_eq!(selected(&sel), vec!["a.sprx"]);
    assert!(sel.pruned.is_empty());
}

#[test]
fn a_self_provided_namespace_does_not_prune() {
    // The module imports the library it also exports; the provider
    // index resolves it to the module itself.
    let cands = candidates(&[(
        "a.sprx",
        make_test_prx_graph_node("modaaaa", "libaaaa", Some("libaaaa")),
    )]);
    let sel = select_import_closure(&cands, None).expect("select");
    assert_eq!(selected(&sel), vec!["a.sprx"]);
}

#[test]
fn no_roots_selects_every_viable_candidate() {
    let cands = candidates(&[
        (
            "a.sprx",
            make_test_prx_graph_node("modaaaa", "libaaaa", None),
        ),
        (
            "b.sprx",
            make_test_prx_graph_node("modbbbb", "libbbbb", None),
        ),
    ]);
    let sel = select_import_closure(&cands, None).expect("select");
    assert_eq!(selected(&sel), vec!["a.sprx", "b.sprx"]);
}
