//! Dependency graph + Kahn topological sort over `BTreeSet`.
//!
//! The graph itself is built inline in
//! [`super::load_firmware_set`]; this module owns only the
//! per-module identity ([`PrxModuleId`], [`module_id_from_name`]),
//! the graph type ([`DependencyGraph`]), and the sort
//! ([`topological_sort`]). Cycle attribution uses Tarjan's
//! strongly-connected-components pass so the
//! `CyclicDependency.involved` field names only nodes that are
//! actually in a cycle, never their innocent downstream consumers.

use std::collections::{BTreeMap, BTreeSet};

use super::PrxLoaderError;

/// Stable per-module identity derived from PRX-header module-name bytes
/// via FNV-1a-32. Two runs over the same firmware install produce the
/// same ids; cross-build-stable contributors to `sync_state_hash`
/// must read the `pub u32` field and feed `to_be_bytes()` into the
/// workspace FNV-1a routine rather than `std::hash::Hash`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PrxModuleId(pub u32);

impl std::fmt::Display for PrxModuleId {
    /// `mod#0xHHHHHHHH` -- compact, sortable, and distinguishable
    /// from a raw u32 in error messages.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mod#{:#010x}", self.0)
    }
}

/// Topologically sorted dependency closure for a firmware-PRX set.
#[derive(Debug)]
pub struct DependencyGraph {
    /// Topological order; consumed by [`super::start_modules`].
    pub order: Vec<PrxModuleId>,
    /// `prerequisite -> set of modules that depend on it`. An entry
    /// `m -> {a, b}` means `m` precedes both `a` and `b` in `order`.
    /// `BTreeSet` for deterministic iteration during Kahn's algorithm.
    pub edges: BTreeMap<PrxModuleId, BTreeSet<PrxModuleId>>,
}

/// Kahn's algorithm over a `BTreeSet` of in-degree-zero nodes.
/// Iteration order of the set determines tie-breaking, which makes
/// the topological order a pure function of the input edge set.
///
/// # Cycle attribution
///
/// On failure, `CyclicDependency.involved` contains only nodes that
/// participate in a cycle: members of any strongly-connected
/// component of size >= 2, plus any node with a self-loop. Innocent
/// downstream consumers of cycle members (e.g., `C` in `A -> B -> A,
/// A -> C`) are NOT reported.
///
/// # Self-imports
///
/// Self-imports must be filtered by the graph builder before they
/// reach this function -- a self-edge here is treated as a cycle.
/// [`super::load_firmware_set`] applies this filter; the test
/// `topological_sort_self_loop_is_a_cycle` documents the policy at
/// the sort layer.
pub fn topological_sort(
    edges: &BTreeMap<PrxModuleId, BTreeSet<PrxModuleId>>,
) -> Result<DependencyGraph, PrxLoaderError> {
    // in_degree[node] = number of modules that depend on `node`.
    let mut in_degree: BTreeMap<PrxModuleId, usize> =
        edges.keys().map(|id| (*id, 0usize)).collect();
    for deps in edges.values() {
        for dep in deps {
            *in_degree.entry(*dep).or_insert(0) += 1;
        }
    }

    let mut ready: BTreeSet<PrxModuleId> = in_degree
        .iter()
        .filter_map(|(id, n)| (*n == 0).then_some(*id))
        .collect();
    let mut order: Vec<PrxModuleId> = Vec::with_capacity(in_degree.len());

    while let Some(id) = ready.pop_first() {
        order.push(id);
        if let Some(deps) = edges.get(&id) {
            for dep in deps {
                if let Some(d) = in_degree.get_mut(dep) {
                    *d -= 1;
                    if *d == 0 {
                        ready.insert(*dep);
                    }
                }
            }
        }
    }

    if order.len() != in_degree.len() {
        // Some nodes never reached in-degree zero. Run a Tarjan SCC
        // pass and pick out the actual cycle participants: SCCs of
        // size >= 2, plus singletons that loop back to themselves.
        let sccs = strongly_connected_components(edges);
        let cycle_members: BTreeSet<PrxModuleId> = sccs
            .iter()
            .filter(|scc| is_cycle_scc(scc, edges))
            .flatten()
            .copied()
            .collect();
        let involved: Vec<PrxModuleId> = cycle_members.into_iter().collect();
        return Err(PrxLoaderError::CyclicDependency { involved });
    }

    Ok(DependencyGraph {
        order,
        edges: edges.clone(),
    })
}

/// `scc` is a cycle iff it has >= 2 nodes (mutual reachability) or
/// it is a singleton with a self-edge.
fn is_cycle_scc(scc: &[PrxModuleId], edges: &BTreeMap<PrxModuleId, BTreeSet<PrxModuleId>>) -> bool {
    debug_assert!(
        !scc.is_empty(),
        "Tarjan invariant: SCCs are non-empty by construction"
    );
    if scc.len() >= 2 {
        return true;
    }
    let n = scc[0];
    edges.get(&n).is_some_and(|deps| deps.contains(&n))
}

/// Tarjan's strongly-connected-components algorithm. Iteration
/// order is fixed by the `BTreeMap` / `BTreeSet` containers, so
/// returned SCCs (and members within each SCC) are deterministic
/// across runs.
fn strongly_connected_components(
    edges: &BTreeMap<PrxModuleId, BTreeSet<PrxModuleId>>,
) -> Vec<Vec<PrxModuleId>> {
    struct State<'a> {
        edges: &'a BTreeMap<PrxModuleId, BTreeSet<PrxModuleId>>,
        index_counter: usize,
        stack: Vec<PrxModuleId>,
        on_stack: BTreeSet<PrxModuleId>,
        indices: BTreeMap<PrxModuleId, usize>,
        lowlinks: BTreeMap<PrxModuleId, usize>,
        sccs: Vec<Vec<PrxModuleId>>,
    }

    impl<'a> State<'a> {
        fn strongconnect(&mut self, v: PrxModuleId) {
            self.indices.insert(v, self.index_counter);
            self.lowlinks.insert(v, self.index_counter);
            self.index_counter += 1;
            self.stack.push(v);
            self.on_stack.insert(v);

            if let Some(succs) = self.edges.get(&v) {
                for &w in succs {
                    if !self.indices.contains_key(&w) {
                        self.strongconnect(w);
                        let lw = self.lowlinks[&w];
                        let lv = self.lowlinks[&v];
                        self.lowlinks.insert(v, lv.min(lw));
                    } else if self.on_stack.contains(&w) {
                        let iw = self.indices[&w];
                        let lv = self.lowlinks[&v];
                        self.lowlinks.insert(v, lv.min(iw));
                    }
                }
            }

            if self.lowlinks[&v] == self.indices[&v] {
                let mut scc = Vec::new();
                loop {
                    let w = self
                        .stack
                        .pop()
                        .expect("strongconnect invariant: stack non-empty at SCC root");
                    self.on_stack.remove(&w);
                    scc.push(w);
                    if w == v {
                        break;
                    }
                }
                self.sccs.push(scc);
            }
        }
    }

    let mut state = State {
        edges,
        index_counter: 0,
        stack: Vec::new(),
        on_stack: BTreeSet::new(),
        indices: BTreeMap::new(),
        lowlinks: BTreeMap::new(),
        sccs: Vec::new(),
    };
    for &v in edges.keys() {
        if !state.indices.contains_key(&v) {
            state.strongconnect(v);
        }
    }
    state.sccs
}

/// FNV-1a-32 over the UTF-8 bytes of `name`. Empty input returns the
/// FNV offset basis `0x811c9dc5`.
///
/// # Collision note
///
/// FNV-1a-32's codomain is the full `u32`, so a non-empty input can
/// statistically collide with the offset basis (a 1-in-2^32 event).
/// Code that relies on "the offset basis means no name" must enforce
/// non-emptiness at parse time (see `parse_prx`'s
/// `PrxParseError::NoModuleInfo` on an empty module-name field) --
/// the hash output alone is not a structural sentinel. The
/// [`super::SYNTHETIC_GAME_ELF_ID`] constant uses the offset basis
/// because `parse_prx` rejects empty names, not because the hash
/// function reserves it.
pub fn module_id_from_name(name: &str) -> PrxModuleId {
    const FNV_OFFSET: u32 = 0x811c_9dc5;
    const FNV_PRIME: u32 = 0x0100_0193;
    let mut h: u32 = FNV_OFFSET;
    for b in name.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(FNV_PRIME);
    }
    PrxModuleId(h)
}

#[cfg(test)]
mod tests {
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
}
