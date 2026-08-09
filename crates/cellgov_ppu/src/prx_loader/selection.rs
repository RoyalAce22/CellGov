//! Import-closure selection over a candidate firmware set.
//!
//! Decides *which* modules load; [`super::body::load_firmware_set`]
//! decides how. Selection and load agree on shadowing because both
//! pick a namespace's provider first-wins in sorted path order over
//! the modules present.
//!
//! Viability is a fixpoint: a module whose import names a namespace
//! with no surviving provider (and not on the permitted-missing list)
//! is dropped, and its own exports stop providing, which can cascade.
//! Pruning is reported, never silent -- a pruned module is a module
//! the boot will not have, and the caller decides whether that is
//! acceptable.

use std::collections::{BTreeMap, BTreeSet};

use super::body::is_permitted_missing;
use super::PrxLoaderError;

/// Why a candidate was dropped from the selection.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PruneReason {
    /// The module imports a namespace with no surviving provider.
    #[error("import namespace {0:?} has no viable provider")]
    UnprovidedImport(String),
    /// The module's relocations span more than the two supported
    /// segments; the loader would reject it.
    #[error("multi-segment relocations (unsupported)")]
    MultiSegmentRelocations,
    /// Another candidate carries the same file-level module name.
    /// Retail firmware ships such alternates (`libac3dec.sprx` /
    /// `libac3dec2.sprx`); a title loads one or the other by path,
    /// never both, so selection keeps the first in path order.
    #[error("module identity already provided by {kept}")]
    DuplicateModuleIdentity {
        /// Candidate that owns the identity.
        kept: String,
    },
}

/// What [`select_import_closure`] chose and what it had to drop.
#[derive(Debug)]
pub struct ClosureSelection {
    /// Candidate paths to load, closed under imports.
    pub selected: BTreeSet<String>,
    /// Dropped modules with the reason each was dropped.
    pub pruned: Vec<(String, PruneReason)>,
    /// Root namespaces no viable candidate provides. Not an error:
    /// the caller's own unresolved-import handling covers their NIDs.
    pub unprovided_roots: BTreeSet<String>,
}

/// Per-candidate parse product the walk runs on.
struct Candidate {
    exports: BTreeSet<String>,
    imports: BTreeSet<String>,
}

/// Select the subset of `candidates` a boot should load.
///
/// `roots` is the set of namespaces the title's own import table
/// names; the selection is their provider closure. `None` selects
/// every viable candidate -- the policy for a firmware executable,
/// whose import tables are built at runtime and name no roots
/// statically.
///
/// # Errors
///
/// [`PrxLoaderError::CandidateParseFailed`] when a candidate does not
/// parse as a PRX. A file that cannot parse cannot load, and dropping
/// it silently would turn a corrupt install into a smaller boot.
pub fn select_import_closure(
    candidates: &BTreeMap<String, Vec<u8>>,
    roots: Option<&BTreeSet<String>>,
) -> Result<ClosureSelection, PrxLoaderError> {
    let mut parsed: BTreeMap<&str, Candidate> = BTreeMap::new();
    let mut pruned: Vec<(String, PruneReason)> = Vec::new();
    let mut id_owner: BTreeMap<super::PrxModuleId, &str> = BTreeMap::new();
    for (path, bytes) in candidates {
        let prx = crate::sprx::parse_prx(bytes).map_err(|source| {
            PrxLoaderError::CandidateParseFailed {
                path: path.clone(),
                source,
            }
        })?;
        if let Some(&kept) = id_owner.get(&prx.module_id) {
            pruned.push((
                path.clone(),
                PruneReason::DuplicateModuleIdentity {
                    kept: kept.to_string(),
                },
            ));
            continue;
        }
        // Known-deferred loader gap, not a corrupt file: the module
        // parses but the loader would reject it, so it is dropped the
        // same way an unsatisfiable import drops a module. Any other
        // loadability failure stays a hard error.
        match super::body::check_loadable(bytes) {
            Ok(()) => {}
            Err(PrxLoaderError::MultiSegmentRelocations { .. }) => {
                pruned.push((path.clone(), PruneReason::MultiSegmentRelocations));
                continue;
            }
            Err(e) => return Err(e),
        }
        let imports = match crate::prx::parse_imports(bytes) {
            Ok(v) => v.into_iter().map(|m| m.name).collect(),
            Err(crate::prx::ImportParseError::NoImportsTable) => BTreeSet::new(),
            Err(source) => {
                return Err(PrxLoaderError::ImportTableParseFailed {
                    module: prx.module_id,
                    source,
                });
            }
        };
        id_owner.insert(prx.module_id, path);
        parsed.insert(
            path,
            Candidate {
                exports: prx.exports.iter().map(|lib| lib.name.clone()).collect(),
                imports,
            },
        );
    }

    // Viability fixpoint. Providers are recomputed per round so a
    // pruned provider hands the namespace to the next exporter in
    // path order, mirroring what load-order shadowing would have done
    // had the pruned module never been present.
    let mut viable: BTreeSet<&str> = parsed.keys().copied().collect();
    loop {
        let provider = provider_index(&parsed, &viable);
        let mut dropped = false;
        for path in viable.clone() {
            let unsatisfied = parsed[path]
                .imports
                .iter()
                .find(|ns| !is_permitted_missing(ns) && !provider.contains_key(ns.as_str()));
            if let Some(ns) = unsatisfied {
                pruned.push((path.to_string(), PruneReason::UnprovidedImport(ns.clone())));
                viable.remove(path);
                dropped = true;
            }
        }
        if !dropped {
            break;
        }
    }

    let provider = provider_index(&parsed, &viable);
    let selected: BTreeSet<String> = match roots {
        None => viable.iter().map(|p| (*p).to_string()).collect(),
        Some(roots) => {
            let mut selected: BTreeSet<&str> = BTreeSet::new();
            let mut queue: Vec<&str> = roots
                .iter()
                .filter_map(|ns| provider.get(ns.as_str()).copied())
                .collect();
            while let Some(path) = queue.pop() {
                if !selected.insert(path) {
                    continue;
                }
                for ns in &parsed[path].imports {
                    if let Some(&dep) = provider.get(ns.as_str()) {
                        if !selected.contains(dep) {
                            queue.push(dep);
                        }
                    }
                }
            }
            selected.iter().map(|p| (*p).to_string()).collect()
        }
    };

    let unprovided_roots = roots
        .map(|roots| {
            roots
                .iter()
                .filter(|ns| !provider.contains_key(ns.as_str()))
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    Ok(ClosureSelection {
        selected,
        pruned,
        unprovided_roots,
    })
}

/// namespace -> first viable path exporting it, in sorted path order.
fn provider_index<'a>(
    parsed: &'a BTreeMap<&'a str, Candidate>,
    viable: &BTreeSet<&'a str>,
) -> BTreeMap<&'a str, &'a str> {
    let mut provider: BTreeMap<&str, &str> = BTreeMap::new();
    for (&path, cand) in parsed {
        if !viable.contains(path) {
            continue;
        }
        for ns in &cand.exports {
            provider.entry(ns.as_str()).or_insert(path);
        }
    }
    provider
}

#[cfg(test)]
#[path = "tests/selection_tests.rs"]
mod tests;
