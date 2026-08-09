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
fn a_duplicate_module_identity_is_pruned_keeping_the_first_path() {
    // Same module name in both files -> same file-level identity.
    // The first in path order owns it; the later one is pruned with
    // the owner named, and its exports stop providing.
    let cands = candidates(&[
        (
            "a.sprx",
            make_test_prx_graph_node("modaaaa", "libaaaa", None),
        ),
        (
            "z.sprx",
            make_test_prx_graph_node("modaaaa", "libzzzz", None),
        ),
    ]);
    let sel = select_import_closure(&cands, None).expect("select");
    assert_eq!(selected(&sel), vec!["a.sprx"]);
    assert_eq!(
        sel.pruned,
        vec![(
            "z.sprx".to_string(),
            super::PruneReason::DuplicateModuleIdentity {
                kept: "a.sprx".to_string(),
            }
        )]
    );

    // The dropped duplicate's library has no provider: a root naming
    // it must be reported unprovided, not silently resolved.
    let sel = select_import_closure(&cands, Some(&roots(&["libzzzz"]))).expect("select");
    assert!(sel.selected.is_empty());
    assert_eq!(sel.unprovided_roots, roots(&["libzzzz"]));
}

#[test]
fn a_corrupt_candidate_is_a_hard_error_naming_the_path() {
    // "a.sprx" sorts before "bad.sprx", so the good candidate parses
    // first; the error must still name the corrupt file, not the
    // first path in the scan.
    let cands = candidates(&[
        (
            "a.sprx",
            make_test_prx_graph_node("modaaaa", "libaaaa", None),
        ),
        ("bad.sprx", b"not a prx".to_vec()),
    ]);
    let err = select_import_closure(&cands, None).unwrap_err();
    match err {
        crate::prx_loader::PrxLoaderError::CandidateParseFailed { path, .. } => {
            assert_eq!(path, "bad.sprx");
        }
        other => panic!("expected CandidateParseFailed, got {other:?}"),
    }
}

#[test]
fn a_multi_segment_relocation_candidate_is_pruned_not_fatal() {
    // RELA r_info lives at file 0x3F8 in the fixture; its high half
    // is the packed segment field (sym), so writing 2 there makes
    // the first relocation target segment 2 -- the loader-rejected
    // multi-segment case.
    let mut multiseg = make_test_prx_graph_node("modcccc", "libcccc", None);
    multiseg[0x3F8..0x3FC].copy_from_slice(&2u32.to_be_bytes());
    let cands = candidates(&[
        (
            "a.sprx",
            make_test_prx_graph_node("modaaaa", "libaaaa", None),
        ),
        ("c.sprx", multiseg),
    ]);
    let sel = select_import_closure(&cands, None).expect("select");
    assert_eq!(selected(&sel), vec!["a.sprx"]);
    assert_eq!(
        sel.pruned,
        vec![(
            "c.sprx".to_string(),
            super::PruneReason::MultiSegmentRelocations
        )]
    );
}

#[test]
fn empty_candidates_select_nothing_and_report_every_root_unprovided() {
    let cands: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let sel = select_import_closure(&cands, None).expect("select");
    assert!(sel.selected.is_empty());
    assert!(sel.pruned.is_empty());
    assert!(sel.unprovided_roots.is_empty());

    let sel = select_import_closure(&cands, Some(&roots(&["libaaaa"]))).expect("select");
    assert!(sel.selected.is_empty());
    assert_eq!(sel.unprovided_roots, roots(&["libaaaa"]));
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
