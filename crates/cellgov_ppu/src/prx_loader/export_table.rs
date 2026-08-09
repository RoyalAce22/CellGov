//! `(namespace, NID)`-to-OPD index over the per-module
//! [`crate::sprx::LoadedPrx::exports`] source of truth.
//!
//! An import names the library it wants, so the library name is half
//! the key. Two modules exporting one NID under different library
//! names each resolve to their own exporter; the firmware corpus
//! relies on this, and a NID-only key cannot express it.
//!
//! Within one library name, the first module in `order` records the
//! authoritative OPD address. A later module exporting the same NID
//! under that same name must agree on the address; disagreement
//! surfaces as [`super::PrxLoaderError::ConflictingExport`] naming
//! the library, with the recording module as `first` and the
//! disagreeing module as `second`. The agreement branch (same NID,
//! same OPD) is a defensive silent no-op; shipping SPRX layouts do
//! not produce it, since exports point into the exporter's own text
//! segment.
//!
//! `build` requires `order` to be a permutation of `loaded.keys()`:
//! a duplicate id in `order` surfaces as
//! [`super::PrxLoaderError::DuplicateModuleInOrder`], and a set
//! mismatch surfaces as [`super::PrxLoaderError::OrderLoadedMismatch`].
//! Both precondition checks fail fast so a downstream "unresolved
//! NID at bind time" cannot mask an upstream contract break.

use std::collections::{BTreeMap, BTreeSet};

use crate::sprx::LoadedPrx;

use super::{PrxLoaderError, PrxModuleId};

/// Library name -> NID -> (relocated OPD guest address, originating
/// module). The origin is retained inside the entry so the
/// `ConflictingExport` error reports the recorder structurally rather
/// than via a parallel-map invariant.
///
/// Nested rather than keyed on a `(String, u32)` tuple so a lookup
/// can borrow the namespace as `&str` without allocating a key.
#[derive(Debug, Default)]
pub struct FirmwareExportTable {
    entries: BTreeMap<String, BTreeMap<u32, (u64, PrxModuleId)>>,
}

impl FirmwareExportTable {
    /// Walk each module's exports in `order` and record the first
    /// OPD address for each NID.
    ///
    /// # Preconditions
    ///
    /// - `order` is a permutation of `loaded.keys()`.
    ///
    /// # Errors
    ///
    /// - [`PrxLoaderError::DuplicateModuleInOrder`] if `order` lists
    ///   any id twice.
    /// - [`PrxLoaderError::OrderLoadedMismatch`] if `order` and
    ///   `loaded.keys()` are not the same set.
    /// - [`PrxLoaderError::ConflictingExport`] if two modules export
    ///   the same NID *under the same library name* to different OPD
    ///   addresses. The same NID under two different library names is
    ///   not a conflict -- that is the case this key exists to
    ///   separate. Same name, same NID, same address is treated as
    ///   agreement and silently kept; see the module-level note.
    pub fn build(
        loaded: &BTreeMap<PrxModuleId, LoadedPrx>,
        order: &[PrxModuleId],
    ) -> Result<Self, PrxLoaderError> {
        // Precondition 1: `order` is duplicate-free.
        let mut seen: BTreeMap<PrxModuleId, usize> = BTreeMap::new();
        for (idx, id) in order.iter().enumerate() {
            if let Some(&first_index) = seen.get(id) {
                return Err(PrxLoaderError::DuplicateModuleInOrder {
                    id: *id,
                    first_index,
                    second_index: idx,
                });
            }
            seen.insert(*id, idx);
        }

        // Precondition 2: `order` and `loaded.keys()` are the same set.
        let order_set: BTreeSet<PrxModuleId> = order.iter().copied().collect();
        let loaded_set: BTreeSet<PrxModuleId> = loaded.keys().copied().collect();
        if order_set != loaded_set {
            return Err(PrxLoaderError::OrderLoadedMismatch {
                in_order_not_loaded: order_set.difference(&loaded_set).copied().collect(),
                in_loaded_not_order: loaded_set.difference(&order_set).copied().collect(),
            });
        }

        let mut entries: BTreeMap<String, BTreeMap<u32, (u64, PrxModuleId)>> = BTreeMap::new();
        for module_id in order {
            // Precondition 2 guarantees every order entry is in loaded.
            let prx = &loaded[module_id];
            for (namespace, by_nid) in &prx.exports {
                let slot = entries.entry(namespace.clone()).or_default();
                for (&nid, &opd) in by_nid {
                    match slot.get(&nid) {
                        None => {
                            slot.insert(nid, (opd, *module_id));
                        }
                        Some(&(existing, _)) if existing == opd => {
                            // Defensive: shipping SPRX doesn't produce
                            // this, but two modules pointing at the same
                            // OPD is logically agreement, not a conflict.
                        }
                        Some(&(_, first)) => {
                            return Err(PrxLoaderError::ConflictingExport {
                                namespace: namespace.clone(),
                                nid,
                                first,
                                second: *module_id,
                            });
                        }
                    }
                }
            }
        }
        Ok(Self { entries })
    }

    /// Lookup an export by the library name the importer asked for.
    pub fn get(&self, namespace: &str, nid: u32) -> Option<u64> {
        self.entries.get(namespace)?.get(&nid).map(|&(opd, _)| opd)
    }

    /// Every library exporting `nid`, for callers holding a NID with
    /// no namespace to pair it with.
    ///
    /// Returns all matches rather than the first: 118 NIDs in the
    /// retail firmware corpus are exported by two or three libraries,
    /// so picking one silently resolves to the wrong function in a
    /// measurable fraction of cases. A caller that cannot disambiguate
    /// should say so rather than guess.
    ///
    /// O(namespaces) -- this walks every library, unlike [`Self::get`].
    pub fn get_any_by_nid(&self, nid: u32) -> Vec<(&str, u64)> {
        self.entries
            .iter()
            .filter_map(|(ns, by_nid)| by_nid.get(&nid).map(|&(opd, _)| (ns.as_str(), opd)))
            .collect()
    }

    /// Number of distinct `(namespace, NID)` pairs recorded.
    pub fn len(&self) -> usize {
        self.entries.values().map(BTreeMap::len).sum()
    }

    /// `true` iff no exports are recorded.
    pub fn is_empty(&self) -> bool {
        self.entries.values().all(BTreeMap::is_empty)
    }

    /// Iterate every recorded `(namespace, NID)` pair. Used by
    /// bidirectional-consistency tests that need a key-only view of
    /// the table without exposing internal origin tracking.
    pub fn keys(&self) -> impl Iterator<Item = (&str, u32)> + '_ {
        self.entries
            .iter()
            .flat_map(|(ns, by_nid)| by_nid.keys().map(move |&nid| (ns.as_str(), nid)))
    }
}

#[cfg(test)]
#[path = "tests/export_table_tests.rs"]
mod tests;
