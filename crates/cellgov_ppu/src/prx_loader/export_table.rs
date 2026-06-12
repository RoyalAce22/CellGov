//! NID-to-OPD index over the per-module [`crate::sprx::LoadedPrx::exports`]
//! source of truth.
//!
//! The first module in `order` that exports a given NID records the
//! authoritative OPD address. A later module exporting the same NID
//! must agree on the address; disagreement surfaces as
//! [`super::PrxLoaderError::ConflictingExport`] with the recording
//! module as `first` and the disagreeing module as `second`. The
//! agreement branch (same NID, same OPD) is a defensive silent
//! no-op; shipping SPRX layouts do not produce it, since exports
//! point into the exporter's own text segment.
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

/// NID -> (relocated OPD guest address, originating module). The
/// origin is retained inside the entry so the `ConflictingExport`
/// error reports the recorder structurally rather than via a
/// parallel-map invariant.
#[derive(Debug, Default)]
pub struct FirmwareExportTable {
    entries: BTreeMap<u32, (u64, PrxModuleId)>,
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
    ///   the same NID to different OPD addresses. Two modules
    ///   exporting the same NID to the same address is treated as
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

        let mut entries: BTreeMap<u32, (u64, PrxModuleId)> = BTreeMap::new();
        for module_id in order {
            // Precondition 2 guarantees every order entry is in loaded.
            let prx = &loaded[module_id];
            for (&nid, &opd) in &prx.exports {
                match entries.get(&nid) {
                    None => {
                        entries.insert(nid, (opd, *module_id));
                    }
                    Some(&(existing, _)) if existing == opd => {
                        // Defensive: shipping SPRX doesn't produce
                        // this, but two modules pointing at the same
                        // OPD is logically agreement, not a conflict.
                    }
                    Some(&(_, first)) => {
                        return Err(PrxLoaderError::ConflictingExport {
                            nid,
                            first,
                            second: *module_id,
                        });
                    }
                }
            }
        }
        Ok(Self { entries })
    }

    /// Lookup an export by NID.
    pub fn get(&self, nid: u32) -> Option<u64> {
        self.entries.get(&nid).map(|&(opd, _)| opd)
    }

    /// Number of distinct NIDs recorded.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` iff no NIDs are recorded.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate every recorded NID. Used by bidirectional-consistency
    /// tests that need a key-only view of the table without exposing
    /// internal origin tracking.
    pub fn nids(&self) -> impl Iterator<Item = u32> + '_ {
        self.entries.keys().copied()
    }
}

#[cfg(test)]
#[path = "tests/export_table_tests.rs"]
mod tests;
