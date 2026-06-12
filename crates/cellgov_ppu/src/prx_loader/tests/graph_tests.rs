//! Module-id hash stability and deterministic topological sort with exact cycle attribution.

use super::*;

#[test]
fn same_name_yields_same_id() {
    assert_eq!(module_id_from_name("liblv2"), module_id_from_name("liblv2"));
}

#[test]
fn different_names_yield_different_ids() {
    assert_ne!(module_id_from_name("liblv2"), module_id_from_name("libfs"));
    assert_ne!(module_id_from_name("liblv2"), module_id_from_name("liblv3"));
}

#[test]
fn empty_name_yields_fnv_offset() {
    // FNV-1a starts the hash at the offset basis and folds
    // zero input bytes through zero rounds -- "" hashes to the
    // basis itself. This is a property of the algorithm, not a
    // reserved sentinel; see the function's collision note.
    assert_eq!(module_id_from_name("").0, 0x811c_9dc5);
}

#[test]
fn name_byte_order_matters() {
    assert_ne!(module_id_from_name("ab"), module_id_from_name("ba"));
}

#[test]
fn module_id_golden_values_for_canonical_min_viable_prx_names() {
    // FNV-1a-32 over the canonical minimum-viable PRX stem names.
    // These values are part of the determinism contract: the
    // sync_state_hash, the loader's dependency graph, and any
    // future trace record consuming module ids all transitively
    // depend on byte-stability of these mappings. Drift in any
    // direction (different hash, different seed, different
    // byte handling) shows up here first. Reviewers can verify
    // these constants by computing FNV-1a-32 over the ASCII
    // bytes of each name with offset 0x811c9dc5, prime
    // 0x01000193.
    assert_eq!(module_id_from_name("liblv2").0, 0xdef6_ed90);
    assert_eq!(module_id_from_name("libfs").0, 0x3f74_3bf7);
    assert_eq!(module_id_from_name("libsysmodule").0, 0xf5d8_93a5);
    assert_eq!(module_id_from_name("libgcm_sys").0, 0xc54f_5a7f);
}

#[test]
fn module_id_round_trips_through_parse_prx() {
    // Import-graph edges are built by hashing the
    // importer-declared name via `module_id_from_name`.
    // Divergence between this hash and parse_prx's derivation
    // would turn every edge into a phantom prerequisite or a
    // missing dependency.
    let bytes = crate::sprx::test_fixtures::make_test_prx();
    let parsed = crate::sprx::parse_prx(&bytes).expect("parse");
    assert_eq!(parsed.module_id, module_id_from_name(&parsed.name));
}

#[test]
fn display_renders_as_mod_hash() {
    assert_eq!(format!("{}", PrxModuleId(0x814c_2d09)), "mod#0x814c2d09");
}

fn id(n: u32) -> PrxModuleId {
    PrxModuleId(n)
}

fn edges_of(pairs: &[(u32, &[u32])]) -> BTreeMap<PrxModuleId, BTreeSet<PrxModuleId>> {
    let mut m: BTreeMap<PrxModuleId, BTreeSet<PrxModuleId>> = BTreeMap::new();
    for (prereq, dependents) in pairs {
        for d in *dependents {
            m.entry(id(*d)).or_default();
        }
        m.entry(id(*prereq))
            .or_default()
            .extend(dependents.iter().map(|d| id(*d)));
    }
    m
}

#[test]
fn topological_sort_linear_chain() {
    // 1 -> 2 -> 3 (1 precedes 2, 2 precedes 3).
    let edges = edges_of(&[(1, &[2]), (2, &[3])]);
    let g = topological_sort(&edges).expect("sort");
    assert_eq!(g.order, vec![id(1), id(2), id(3)]);
}

#[test]
fn topological_sort_diamond_obeys_partial_order() {
    // 1 -> 2, 1 -> 3, 2 -> 4, 3 -> 4.
    let edges = edges_of(&[(1, &[2, 3]), (2, &[4]), (3, &[4])]);
    let g = topological_sort(&edges).expect("sort");
    let pos = |x: u32| g.order.iter().position(|i| *i == id(x)).unwrap();
    assert!(pos(1) < pos(2));
    assert!(pos(1) < pos(3));
    assert!(pos(2) < pos(4));
    assert!(pos(3) < pos(4));
}

#[test]
fn topological_sort_breaks_ties_by_module_id_for_determinism() {
    let edges = edges_of(&[(1, &[2, 3]), (2, &[4]), (3, &[4])]);
    let g = topological_sort(&edges).expect("sort");
    // 2 has smaller id than 3, both become ready after 1; the
    // BTreeSet pops the smaller first.
    assert_eq!(g.order, vec![id(1), id(2), id(3), id(4)]);
}

#[test]
fn topological_sort_cycle_returns_error_with_exact_involved_set() {
    // 1 -> 2 -> 1.
    let edges = edges_of(&[(1, &[2]), (2, &[1])]);
    let err = topological_sort(&edges).unwrap_err();
    let PrxLoaderError::CyclicDependency { involved } = err else {
        panic!("expected CyclicDependency");
    };
    assert_eq!(involved.len(), 2, "involved must be exactly the cycle");
    assert!(involved.contains(&id(1)));
    assert!(involved.contains(&id(2)));
}

#[test]
fn topological_sort_cycle_excludes_innocent_downstream_nodes() {
    // 1 -> 2 -> 1, 1 -> 3. Node 3 depends on 1 but is not part
    // of the cycle; only the SCC members 1 and 2 should appear
    // in `involved`.
    let edges = edges_of(&[(1, &[2, 3]), (2, &[1])]);
    let err = topological_sort(&edges).unwrap_err();
    let PrxLoaderError::CyclicDependency { involved } = err else {
        panic!("expected CyclicDependency");
    };
    assert_eq!(involved, vec![id(1), id(2)]);
    assert!(
        !involved.contains(&id(3)),
        "innocent downstream node leaked into cycle attribution"
    );
}

#[test]
fn topological_sort_self_loop_is_a_cycle() {
    let edges = edges_of(&[(1, &[1])]);
    let err = topological_sort(&edges).unwrap_err();
    let PrxLoaderError::CyclicDependency { involved } = err else {
        panic!("expected CyclicDependency");
    };
    assert_eq!(involved, vec![id(1)]);
}

#[test]
fn topological_sort_singletons_emit_in_id_order() {
    let edges = edges_of(&[(3, &[]), (1, &[]), (2, &[])]);
    let g = topological_sort(&edges).expect("sort");
    assert_eq!(g.order, vec![id(1), id(2), id(3)]);
}

#[test]
fn topological_sort_empty_edges_returns_empty_order() {
    // Locked-down: empty input is not an error.
    let edges: BTreeMap<PrxModuleId, BTreeSet<PrxModuleId>> = BTreeMap::new();
    let g = topological_sort(&edges).expect("sort");
    assert!(g.order.is_empty());
}
